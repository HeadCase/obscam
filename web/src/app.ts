import { deriveWhepUrl, parseRuntimeContract } from "./model.js";
import {
  ControlClient,
  readStoredCredentials,
  type CameraSettings,
  type ControlEvent,
  type ControlState,
  type ControlTransition
} from "./control.js";
import { startWhep, type WhepSession } from "./whep.js";
import { ReconnectLoop } from "./reconnect.js";
import {
  acceptsMediaPresentation,
  initialViewerState,
  parseLifecycleFacts,
  reduceViewer,
  viewerProjection,
  type ViewerEvent,
  type ViewerState
} from "./viewer.js";
import {
  ServiceQualityClient,
  type QualityObservation,
  type ServiceQualitySummary
} from "./service-quality.js";
import { QualityObservationSettler } from "./quality-settling.js";
import {
  advanceExposureRail,
  initialExposureRailState,
  initialOperatorUiState,
  reduceOperatorUi,
  settingPresentationPhase,
  type OperatorUiEvent,
  type OperatorUiState
} from "./operator-ui.js";
import { isSnapshotCancellation, snapshotFilename } from "./snapshot.js";

const LIVENESS_TICK_MS = 100;
const INITIAL_RETRY_MS = 250;
const MAXIMUM_RETRY_MS = 5_000;
const STABLE_CONNECTION_MS = 5_000;
const QUALITY_SETTLING_MS = 1_100;
const EXPOSURE_CHOICES_MS = [
  50, 100, 200, 300, 500, 1_000, 2_000, 5_000, 10_000, 15_000, 20_000, 30_000
] as const;
const ISO_CHOICES = [0, 50, 100, 150, 200, 250, 300, 350, 400, 450, 500, 550, 600] as const;

async function boot(): Promise<void> {
  const viewerRoot = requiredElement("[data-viewer]");
  const viewerFrame = requiredElement("[data-viewer-frame]");
  const status = requiredElement("[data-viewer-status]");
  const detail = requiredElement("[data-viewer-detail]");
  const unavailable = requiredElement("[data-viewer-unavailable]");
  const serviceStatus = requiredElement("[data-service-status]");
  const exposureRail = requiredElement("[data-exposure-rail]");
  const exposureRailFill = requiredElement("[data-exposure-rail-fill]");
  const settingsPopover = requiredElement("[data-settings-popover]");
  const settingsTrigger = requiredButton("[data-control=\"settings\"]");
  const snapshot = requiredButton("[data-control=\"snapshot\"]");
  const announcement = requiredElement("[data-announcement]");
  const video = requiredVideo("[data-viewer-video]");
  const retainedFrame = requiredCanvas("[data-viewer-retained-frame]");
  const takeControl = requiredButton("[data-control=\"take-control\"]");
  const exposure = requiredInput("[data-exposure]");
  const exposureOutput = requiredElement("[data-exposure-output]");
  const gain = requiredInput("[data-gain]");
  const gainOutput = requiredElement("[data-gain-output]");
  const monochrome = requiredButton("[data-control=\"treatment-monochrome\"]");
  const colour = requiredButton("[data-control=\"treatment-colour\"]");
  const applySettings = requiredButton("[data-control=\"apply-settings\"]");
  const discardSettings = requiredButton("[data-control=\"discard-settings\"]");
  const settingUi = {
    exposure: settingElements("exposure"),
    gain: settingElements("gain"),
    treatment: settingElements("treatment")
  };
  const readout = {
    exposure: requiredElement("[data-readout-exposure]"),
    gain: requiredElement("[data-readout-gain]"),
    treatment: requiredElement("[data-readout-treatment]")
  };

  try {
    const response = await fetch("/api/v1/runtime", {
      cache: "no-store",
      headers: { Accept: "application/json" }
    });
    if (!response.ok) {
      throw new Error(`runtime request failed with ${response.status}`);
    }
    const runtime = parseRuntimeContract(await response.json());
    let viewer = initialViewerState(runtime.runtimeEpoch, readStoredCredentials());
    let operatorUi: OperatorUiState = initialOperatorUiState(performance.now());
    let announcementTimer: number | null = null;
    let quality: ServiceQualityClient | null = null;
    let qualityRuntimeEpoch: string | null = null;
    let qualityConnection: Promise<void> | null = null;
    let qualityMediaGeneration = 0;
    let qualitySync: Promise<void> = Promise.resolve();
    let mediaSession: WhepSession | null = null;
    let mediaAttemptCancellation: AbortController | null = null;
    let stopPresentedFrames: (() => void) | null = null;
    let railAnimationFrame: number | null = null;
    let railState = initialExposureRailState();
    let lastReducedMotionRailUpdateAtMs = Number.NEGATIVE_INFINITY;
    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    let mediaReconnect: ReconnectLoop;
    const qualitySettler = new QualityObservationSettler(
      QUALITY_SETTLING_MS,
      {
        schedule: (callback, delayMs) => window.setTimeout(callback, delayMs),
        cancel: (timer) => window.clearTimeout(timer)
      },
      (observation, presentedAtUnixUs) => quality?.report(observation, presentedAtUnixUs)
    );

    const render = (): void => {
      const projection = viewerProjection(viewer);
      const operatorDetail = operatorFeedDetail(projection.status, projection.detail, projection.frameAgeMs);
      status.textContent = projection.status;
      detail.textContent = operatorDetail;
      serviceStatus.textContent = projection.status;
      unavailable.hidden = viewer.trustworthyFrame !== null || projection.status === "Live";
      viewerFrame.dataset.feedState = feedStateToken(projection.status);
      viewerRoot.dataset.chromeVisible = String(operatorUi.chromeVisible);
      settingsPopover.hidden = !operatorUi.settingsOpen || viewer.control.ownership !== "you";
      renderControl(
        viewer.control,
        operatorUi.settingRejectionUntilMs !== null,
        takeControl,
        settingsTrigger,
        snapshot,
        video,
        retainedFrame,
        exposure,
        exposureOutput,
        gain,
        gainOutput,
        monochrome,
        colour,
        applySettings,
        discardSettings,
        settingUi,
        readout
      );
    };

    const animateExposureRail = (): void => {
      const projection = viewerProjection(viewer);
      const capture = viewer.lifecycle?.capture;
      const allowed = capture !== null && capture !== undefined &&
        (projection.status === "Live" || projection.status === "Waiting for first image");
      const pendingGeneration = viewer.control.settings.pending?.generation ?? null;
      const applying = viewer.control.submitted !== null ||
        (pendingGeneration !== null && capture?.settingsGeneration !== pendingGeneration);
      const calibratedNowUnixUs = currentServerUnixUs(
        quality,
        qualityRuntimeEpoch,
        viewer.runtimeEpoch
      );
      const nowMs = performance.now();
      const captureChanged = capture?.startedAtUnixUs !== railState.captureStartedAtUnixUs;
      if (!reducedMotion || captureChanged || nowMs - lastReducedMotionRailUpdateAtMs >= 250) {
        railState = advanceExposureRail(
          railState,
          allowed && calibratedNowUnixUs !== null
            ? {
                exposureMs: capture.exposureMs,
                startedAtUnixUs: capture.startedAtUnixUs,
                nowUnixUs: calibratedNowUnixUs
              }
            : null
        );
        lastReducedMotionRailUpdateAtMs = nowMs;
      }
      exposureRail.hidden = applying || railState.progress === null;
      exposureRailFill.style.width = `${(railState.progress ?? 0) * 100}%`;
      exposureRail.classList.toggle(
        "is-arriving",
        !applying && operatorUi.frameArrivalUntilMs !== null &&
          nowMs < operatorUi.frameArrivalUntilMs
      );
      railAnimationFrame = window.requestAnimationFrame(animateExposureRail);
    };

    const dispatchOperatorUi = (event: OperatorUiEvent): void => {
      const next = reduceOperatorUi(operatorUi, event);
      if (next !== operatorUi) {
        const appearanceChanged =
          next.chromeVisible !== operatorUi.chromeVisible ||
          next.settingsOpen !== operatorUi.settingsOpen ||
          next.settingRejectionUntilMs !== operatorUi.settingRejectionUntilMs;
        operatorUi = next;
        if (appearanceChanged) render();
      }
    };

    const announce = (message: string, kind: "status" | "error" = "status"): void => {
      if (announcementTimer !== null) window.clearTimeout(announcementTimer);
      announcement.textContent = message;
      announcement.dataset.kind = kind;
      announcement.hidden = false;
      if (kind === "status") {
        announcementTimer = window.setTimeout(() => {
          announcement.hidden = true;
          announcementTimer = null;
        }, 2_500);
      }
    };

    const connectQuality = async (): Promise<void> => {
      while (qualityRuntimeEpoch !== viewer.runtimeEpoch) {
        if (qualityConnection === null) {
          const targetEpoch = viewer.runtimeEpoch;
          qualityConnection = ServiceQualityClient.connect(targetEpoch, (evidence) => {
            if (viewer.runtimeEpoch !== targetEpoch) return;
            dispatch({
              type: "evidence",
              p99DeliveryUs: currentDeliveryP99Us(evidence),
              response: evidence
            });
          }).then((client) => {
            if (viewer.runtimeEpoch === targetEpoch) {
              quality = client;
              qualityRuntimeEpoch = targetEpoch;
            }
          }).finally(() => {
            qualityConnection = null;
          });
        }
        await qualityConnection;
      }
    };

    const syncQuality = (mediaGeneration: number): Promise<void> => {
      qualitySync = qualitySync.catch(() => {}).then(async () => {
        if (mediaGeneration <= qualityMediaGeneration) return;
        await connectQuality();
        if (quality === null) return;
        if (qualityMediaGeneration > 0) await quality.reconnect();
        qualityMediaGeneration = mediaGeneration;
      });
      return qualitySync;
    };

    const qualityFailed = (error: unknown): void => {
      console.error("ObsCam service-quality connection failed", error);
    };

    const connectMedia = async (connectionGeneration: number): Promise<void> => {
      qualitySettler.clear();
      mediaAttemptCancellation?.abort();
      const cancellation = new AbortController();
      mediaAttemptCancellation = cancellation;
      dispatch({ type: "media_connecting", connectionGeneration });
      if (
        viewer.trustworthyFrame !== null &&
        retainedFrame.dataset.trustworthyFrame === "true"
      ) {
        retainedFrame.hidden = false;
      }
      stopPresentedFrames?.();
      stopPresentedFrames = null;
      const previous = mediaSession;
      void syncQuality(connectionGeneration).catch(qualityFailed);
      try {
        const session = await startWhep(
          deriveWhepUrl(runtime.media, window.location.href),
          () => {
            if (connectionGeneration !== mediaReconnect.connectionGeneration) return;
            dispatch({ type: "media_disconnected", connectionGeneration });
            mediaReconnect.failed(connectionGeneration);
          },
          cancellation.signal
        );
        if (connectionGeneration !== mediaReconnect.connectionGeneration) {
          await session.close();
          return;
        }
        session.attach(video);
        mediaAttemptCancellation = null;
        mediaSession = session;
        const previousCleanup = previous?.close();
        mediaReconnect.connected(connectionGeneration);
        dispatch({ type: "media_connected", connectionGeneration });
        stopPresentedFrames = watchPresentedFrames(video, (metadata) => {
          presentedFrame(connectionGeneration, metadata);
        });
        await previousCleanup;
      } catch (error: unknown) {
        if (
          connectionGeneration === mediaReconnect.connectionGeneration &&
          !cancellation.signal.aborted
        ) {
          dispatch({ type: "media_disconnected", connectionGeneration });
          console.error("ObsCam WHEP connection failed", error);
          mediaReconnect.failed(connectionGeneration);
        }
      }
    };

    const dispatch = (event: ViewerEvent): ReturnType<typeof reduceViewer> => {
      const transition = reduceViewer(viewer, event);
      viewer = transition.state;
      render();
      if (transition.effects.includes("reconnect_media")) {
        mediaReconnect.retryNow();
      }
      return transition;
    };

    const controlTransition = (event: ControlEvent): ControlTransition => {
      if (event.type === "accepted") markSettingsTiming("server-accepted", {
        settingsGeneration: event.targetGeneration
      });
      if (event.type === "applied") markSettingsTiming("camera-applied", {
        settingsGeneration: event.settingsGeneration
      });
      const priorOwnership = viewer.control.ownership;
      const priorRejected = viewer.control.rejected;
      const transition = dispatch({ type: "control", event });
      const nextOwnership = transition.state.control.ownership;
      if (
        transition.state.control.rejected !== null &&
        transition.state.control.rejected !== priorRejected
      ) {
        dispatchOperatorUi({ type: "setting_rejected", nowMs: performance.now() });
      }
      if (priorOwnership !== "you" && nextOwnership === "you") {
        dispatchOperatorUi({ type: "authority_granted", nowMs: performance.now() });
      } else if (priorOwnership === "you" && nextOwnership !== "you") {
        dispatchOperatorUi({ type: "authority_lost", nowMs: performance.now() });
        announce(
          nextOwnership === "another_viewer"
            ? "Control taken by another viewer"
            : event.type === "released"
              ? "Control released"
              : "Control lost"
        );
      } else if (transition.state.control.notice !== null && event.type !== "intent_queued") {
        announce(operatorControlNotice(transition.state.control.notice));
      }
      return { state: transition.state.control, storage: transition.controlStorage };
    };

    const presentedFrame = (
      mediaConnectionGeneration: number,
      metadata: VideoFrameCallbackMetadata
    ): void => {
      const currentMedia = acceptsMediaPresentation(viewer, mediaConnectionGeneration);
      const nowUnixUs = currentServerUnixUs(quality, qualityRuntimeEpoch, viewer.runtimeEpoch);
      let exactlyMappedStreamEpoch: number | null = null;
      if (nowUnixUs !== null) {
        const transition = dispatch({
          type: "presented",
          mediaConnectionGeneration,
          ...(metadata.rtpTimestamp === undefined ? {} : { rtpTimestamp: metadata.rtpTimestamp }),
          nowUnixUs
        });
        if (transition.presentation !== null && !transition.presentation.mapping.repeat) {
          exactlyMappedStreamEpoch = transition.presentation.mapping.streamEpoch;
          dispatchOperatorUi({ type: "exact_frame_presented", nowMs: performance.now() });
          markSettingsTiming("browser-presented-exact", {
            settingsGeneration: transition.presentation.mapping.settingsGeneration,
            unixUs: transition.presentation.presentedAtUnixUs
          });
        }
      }
      const trustworthy =
        currentMedia &&
        nowUnixUs !== null &&
        !viewer.awaitingCurrentPresentation &&
        viewer.correlationLostAtUnixUs === null &&
        viewer.trustworthyFrame !== null &&
        viewer.trustworthyFrame.streamEpoch === viewer.presentation.streamEpoch;
      if (trustworthy && viewer.trustworthyFrame !== null) {
        const frame = viewer.trustworthyFrame;
        captureTrustworthyFrame(
          video,
          retainedFrame,
          `${frame.runtimeEpoch}:${frame.streamEpoch}:${frame.sourceGeneration}`
        );
        retainedFrame.hidden = true;
      }
      const exact = currentMedia && exactlyMappedStreamEpoch !== null;
      const observation: QualityObservation = {
        correlation: exact ? "exact" : "unknown",
        streamEpoch: exact
          ? exactlyMappedStreamEpoch
          : viewer.presentation.streamEpoch === 0 ? null : viewer.presentation.streamEpoch,
        presentedFrames: metadata.presentedFrames,
        visibility: document.visibilityState === "visible" ? "visible" : "hidden"
      };
      if (nowUnixUs !== null) {
        const exactObservation = exact && metadata.rtpTimestamp !== undefined
          ? { ...observation, rtpTimestamp: metadata.rtpTimestamp }
          : observation;
        qualitySettler.queue(exactObservation, nowUnixUs, metadata.rtpTimestamp);
      }
    };
    const control = new ControlClient(
      () => viewer.runtimeEpoch,
      () => viewer.control,
      controlTransition,
      (mapping) => {
        if (!mapping.repeat) {
          markSettingsTiming("exposure-completed", {
            settingsGeneration: mapping.settingsGeneration,
            unixUs: mapping.exposureCompletedAtUnixUs
          });
          markSettingsTiming("encode-published", {
            settingsGeneration: mapping.settingsGeneration,
            unixUs: mapping.submittedAtUnixUs
          });
        }
        const transition = dispatch({ type: "mapping", mapping });
        qualitySettler.promote(mapping.rtpTimestamp, mapping.streamEpoch);
        if (transition.presentation !== null && !transition.presentation.mapping.repeat) {
          dispatchOperatorUi({ type: "exact_frame_presented", nowMs: performance.now() });
          markSettingsTiming("browser-presented-exact", {
            settingsGeneration: transition.presentation.mapping.settingsGeneration,
            unixUs: transition.presentation.presentedAtUnixUs
          });
        }
      },
      (value) => dispatch({ type: "lifecycle", facts: parseLifecycleFacts(value) })
    );
    mediaReconnect = new ReconnectLoop(
      (connectionGeneration) => void connectMedia(connectionGeneration),
      {
        initialDelayMs: INITIAL_RETRY_MS,
        maximumDelayMs: MAXIMUM_RETRY_MS,
        stableAfterMs: STABLE_CONNECTION_MS
      }
    );

    takeControl.addEventListener("click", () => control.toggleAuthority());
    settingsTrigger.addEventListener("click", () => {
      if (!viewer.control.mayMutate) return;
      dispatchOperatorUi({ type: "settings_toggled", nowMs: performance.now() });
    });
    exposure.addEventListener("input", () => {
      const exposureMs = EXPOSURE_CHOICES_MS[Number(exposure.value)];
      if (exposureMs === undefined) return;
      markSettingsTiming("browser-action", { field: "exposure" });
      dispatch({ type: "control", event: { type: "draft_changed", settings: {
        ...selectedSettings(viewer.control), exposureMs
      } } });
    });
    gain.addEventListener("input", () => {
      const selectedGain = ISO_CHOICES[Number(gain.value)];
      if (selectedGain === undefined) return;
      markSettingsTiming("browser-action", { field: "gain" });
      dispatch({ type: "control", event: { type: "draft_changed", settings: {
        ...selectedSettings(viewer.control),
        gain: selectedGain
      } } });
    });
    monochrome.addEventListener("click", () => {
      markSettingsTiming("browser-action", { field: "treatment" });
      dispatch({ type: "control", event: { type: "draft_changed", settings: {
        ...selectedSettings(viewer.control), treatment: "monochrome"
      } } });
    });
    colour.addEventListener("click", () => {
      markSettingsTiming("browser-action", { field: "treatment" });
      dispatch({ type: "control", event: { type: "draft_changed", settings: {
        ...selectedSettings(viewer.control), treatment: "colour"
      } } });
    });
    applySettings.addEventListener("click", () => {
      if (viewer.control.draft !== null) {
        markSettingsTiming("submitted", { settings: viewer.control.draft });
        control.setSettings(viewer.control.draft);
      }
    });
    discardSettings.addEventListener("click", () => {
      dispatch({ type: "control", event: { type: "draft_discarded" } });
    });
    snapshot.addEventListener("click", () => {
      void saveVisibleSnapshot(video, retainedFrame, viewer)
        .then((result) => {
          if (result !== null) announce(result);
        })
        .catch((error: unknown) => {
          if (!isSnapshotCancellation(error)) {
            console.error("ObsCam snapshot failed", error);
            announce("Snapshot failed · check browser storage permissions", "error");
          }
        });
    });

    viewerRoot.addEventListener("pointermove", () => {
      dispatchOperatorUi({ type: "activity", nowMs: performance.now() });
    });
    viewerRoot.addEventListener("pointerenter", () => {
      dispatchOperatorUi({ type: "activity", nowMs: performance.now() });
    });
    viewerFrame.addEventListener("pointerdown", (event) => {
      if (event.target === viewerFrame || event.target === video || event.target === retainedFrame) {
        event.stopPropagation();
        dispatchOperatorUi({ type: "chrome_toggled", nowMs: performance.now() });
      }
    });
    const setProtection = (
      protection: "hover" | "focus" | "press" | "drag",
      active: boolean
    ): void => {
      dispatchOperatorUi({
        type: "protection_changed",
        protection,
        active,
        nowMs: performance.now()
      });
    };
    const chrome = requiredElement("[data-chrome]");
    chrome.addEventListener("pointerenter", () => setProtection("hover", true));
    chrome.addEventListener("pointerleave", () => setProtection("hover", false));
    chrome.addEventListener("focusin", () => setProtection("focus", true));
    chrome.addEventListener("focusout", (event) => {
      if (!(event.relatedTarget instanceof Node) || !chrome.contains(event.relatedTarget)) {
        setProtection("focus", false);
      }
    });
    chrome.addEventListener("pointerdown", () => setProtection("press", true));
    const clearPointerProtections = (): void => {
      setProtection("press", false);
      setProtection("drag", false);
    };
    window.addEventListener("pointerup", clearPointerProtections);
    window.addEventListener("pointercancel", clearPointerProtections);
    window.addEventListener("blur", clearPointerProtections);
    for (const slider of [exposure, gain]) {
      slider.addEventListener("pointerdown", () => setProtection("drag", true));
    }
    document.addEventListener("pointerdown", (event) => {
      if (
        operatorUi.settingsOpen &&
        event.target instanceof Node &&
        !settingsPopover.contains(event.target) &&
        !settingsTrigger.contains(event.target)
      ) {
        dispatchOperatorUi({ type: "settings_closed", nowMs: performance.now() });
      }
      dispatchOperatorUi({ type: "activity", nowMs: performance.now() });
    });
    document.addEventListener("keydown", (event) => {
      dispatchOperatorUi({ type: "activity", nowMs: performance.now() });
      if (event.key === "Escape" && operatorUi.settingsOpen) {
        dispatchOperatorUi({ type: "settings_closed", nowMs: performance.now() });
        settingsTrigger.focus();
      } else if (
        event.key === "Enter" &&
        operatorUi.settingsOpen &&
        viewer.control.draft !== null &&
        (event.target === exposure || event.target === gain)
      ) {
        applySettings.click();
      }
    });

    render();
    control.start();
    railAnimationFrame = window.requestAnimationFrame(animateExposureRail);

    document.addEventListener("visibilitychange", () => {
      const visible = document.visibilityState === "visible";
      const foreground = visible && viewer.visibility === "hidden";
      if (!visible) {
        mediaAttemptCancellation?.abort();
        mediaAttemptCancellation = null;
        stopPresentedFrames?.();
        stopPresentedFrames = null;
        const hiddenSession = mediaSession;
        mediaSession = null;
        void hiddenSession?.close();
      }
      dispatch({
        type: "visibility",
        visibility: visible ? "visible" : "hidden"
      });
      if (foreground && viewer.control.connection === "disconnected") control.retryNow();
    });
    window.addEventListener("online", () => {
      mediaReconnect.retryNow();
      control.retryNow();
    });
    const tick = window.setInterval(() => {
      dispatchOperatorUi({ type: "tick", nowMs: performance.now() });
      const nowUnixUs = currentServerUnixUs(quality, qualityRuntimeEpoch, viewer.runtimeEpoch);
      if (nowUnixUs !== null) {
        dispatch({ type: "tick", nowUnixUs });
      }
    }, LIVENESS_TICK_MS);
    window.addEventListener("pagehide", () => {
      window.clearInterval(tick);
      if (railAnimationFrame !== null) window.cancelAnimationFrame(railAnimationFrame);
      if (announcementTimer !== null) window.clearTimeout(announcementTimer);
      qualitySettler.clear();
      mediaReconnect.close();
      mediaAttemptCancellation?.abort();
      control.close();
      stopPresentedFrames?.();
      void mediaSession?.close();
    }, { once: true });
    mediaReconnect.start();
  } catch (error: unknown) {
    status.textContent = "Unavailable";
    detail.textContent = "Runtime status unavailable";
    console.error("ObsCam viewer bootstrap failed", error);
  }
}

function markSettingsTiming(name: string, detail: unknown): void {
  performance.mark(`obscam.settings.${name}`, { detail });
}

function currentServerUnixUs(
  quality: ServiceQualityClient | null,
  qualityRuntimeEpoch: string | null,
  runtimeEpoch: string
): number | null {
  return quality !== null && qualityRuntimeEpoch === runtimeEpoch ? quality.nowUnixUs() : null;
}

function currentDeliveryP99Us(response: ServiceQualitySummary): number | null {
  const client = response.clients[0];
  if (client === undefined) return null;
  const candidates = client.partitions
    .filter((partition) =>
      partition.connectionGeneration === client.connectionGeneration &&
      partition.visibility === "visible" &&
      partition.exactCorrelation > 0 &&
      partition.latencyUs !== null
    )
    .map((partition) => partition.latencyUs?.p99 ?? 0);
  return candidates.length === 0 ? null : Math.max(...candidates);
}

function selectedSettings(state: ControlState): CameraSettings {
  return state.draft ?? state.submitted ??
    state.settings.pending?.settings ?? state.settings.applied.settings;
}

function watchPresentedFrames(
  video: HTMLVideoElement,
  presented: (metadata: VideoFrameCallbackMetadata) => void
): () => void {
  let callbackId: number | null = null;
  const callback: VideoFrameRequestCallback = (_now, metadata) => {
    presented(metadata);
    callbackId = video.requestVideoFrameCallback(callback);
  };
  callbackId = video.requestVideoFrameCallback(callback);
  return () => {
    if (callbackId !== null) {
      video.cancelVideoFrameCallback(callbackId);
      callbackId = null;
    }
  };
}

function captureTrustworthyFrame(
  video: HTMLVideoElement,
  retainedFrame: HTMLCanvasElement,
  frameIdentity: string
): void {
  if (retainedFrame.dataset.frameIdentity === frameIdentity) return;
  if (video.videoWidth === 0 || video.videoHeight === 0) return;
  const context = retainedFrame.getContext("2d");
  if (context === null) return;
  retainedFrame.width = video.videoWidth;
  retainedFrame.height = video.videoHeight;
  context.drawImage(video, 0, 0, retainedFrame.width, retainedFrame.height);
  retainedFrame.dataset.trustworthyFrame = "true";
  retainedFrame.dataset.frameIdentity = frameIdentity;
}

function renderControl(
  state: ControlState,
  showRejected: boolean,
  takeControl: HTMLButtonElement,
  settingsTrigger: HTMLButtonElement,
  snapshot: HTMLButtonElement,
  video: HTMLVideoElement,
  retainedFrame: HTMLCanvasElement,
  exposure: HTMLInputElement,
  exposureOutput: HTMLElement,
  gain: HTMLInputElement,
  gainOutput: HTMLElement,
  monochrome: HTMLButtonElement,
  colour: HTMLButtonElement,
  applySettings: HTMLButtonElement,
  discardSettings: HTMLButtonElement,
  settingUi: Record<"exposure" | "gain" | "treatment", SettingElements>,
  readout: Record<"exposure" | "gain" | "treatment", HTMLElement>
): void {
  takeControl.disabled = state.connection !== "connected" || state.pendingIntent;
  takeControl.textContent = state.ownership === "you" ? "Release" : "Take control";
  const settings = selectedSettings(state);
  const settingsDisabled = !state.mayMutate;
  const disabledDescription = "Take control to change settings";
  settingsTrigger.disabled = settingsDisabled;
  settingsTrigger.classList.toggle("has-draft", state.draft !== null);
  describeDisabledControl(settingsTrigger, settingsDisabled, disabledDescription);
  if (!settingsDisabled && state.draft !== null) {
    settingsTrigger.setAttribute("aria-description", "Unapplied changes");
  }
  snapshot.disabled = !snapshotPixelsAvailable(video, retainedFrame);
  exposure.disabled = settingsDisabled;
  describeDisabledControl(exposure, settingsDisabled, disabledDescription);
  exposure.value = String(EXPOSURE_CHOICES_MS.findIndex((value) => value === settings.exposureMs));
  exposure.dataset.exposureMs = String(settings.exposureMs);
  exposure.setAttribute("aria-valuetext", formatExposure(settings.exposureMs));
  exposureOutput.textContent = formatExposure(settings.exposureMs);
  gain.disabled = settingsDisabled;
  describeDisabledControl(gain, settingsDisabled, disabledDescription);
  gain.value = String(ISO_CHOICES.findIndex((value) => value === settings.gain));
  gain.setAttribute("aria-valuetext", `ISO ${settings.gain}`);
  gainOutput.textContent = String(settings.gain);
  monochrome.disabled = settingsDisabled;
  colour.disabled = settingsDisabled;
  describeDisabledControl(monochrome, settingsDisabled, disabledDescription);
  describeDisabledControl(colour, settingsDisabled, disabledDescription);
  monochrome.setAttribute("aria-pressed", String(settings.treatment === "monochrome"));
  colour.setAttribute("aria-pressed", String(settings.treatment === "colour"));
  const draftActionsDisabled = settingsDisabled || state.draft === null;
  applySettings.disabled = draftActionsDisabled || state.pendingIntent;
  discardSettings.disabled = draftActionsDisabled;
  renderSettingState(settingUi.exposure, "exposureMs", state, EXPOSURE_CHOICES_MS, showRejected);
  renderSettingState(settingUi.gain, "gain", state, ISO_CHOICES, showRejected);
  renderSettingState(settingUi.treatment, "treatment", state, null, showRejected);
  renderReadout(readout, state);
}

type SettingsField = keyof CameraSettings;

interface SettingElements {
  shell: HTMLElement;
  semantics: HTMLElement;
  visibleMarker: HTMLElement | null;
  requestedMarker: HTMLElement | null;
}

function renderSettingState(
  elements: SettingElements,
  field: SettingsField,
  state: ControlState,
  choices: readonly number[] | null,
  showRejected: boolean
): void {
  const requested = newestRequestedSettings(state);
  const selected = selectedSettings(state);
  const visible = state.settings.visible?.settings ?? null;
  const drafted = state.draft !== null && state.draft[field] !== requested[field];
  const requestedDiffers = visible === null || requested[field] !== visible[field];
  const rejected = showRejected &&
    state.rejected !== null && state.rejected[field] !== visible?.[field];
  const applied = state.settings.pending !== null &&
    state.settings.applied.generation >= state.settings.pending.generation;
  const awaitingPresentation = state.settings.pending !== null || state.submitted !== null;
  const phase = settingPresentationPhase({
    visibleKnown: visible !== null,
    drafted,
    requestedDiffers,
    rejected,
    applied,
    awaitingPresentation
  });
  elements.shell.dataset.settingState = phase;
  const visibleValue = visible?.[field];
  if (choices !== null && typeof visibleValue === "number") {
    elements.shell.style.setProperty(
      "--visible-ratio",
      String(indexRatio(choices, visibleValue))
    );
  }
  if (elements.visibleMarker !== null) elements.visibleMarker.hidden = visible === null;
  if (choices !== null && typeof requested[field] === "number") {
    elements.shell.style.setProperty(
      "--requested-ratio",
      String(indexRatio(choices, requested[field] as number))
    );
  }
  if (elements.requestedMarker !== null) {
    elements.requestedMarker.hidden = !drafted || !requestedDiffers;
  }
  elements.semantics.textContent = settingSemantics(
    phase,
    field,
    visibleValue,
    selected[field]
  );
}

function settingSemantics(
  phase: string,
  field: SettingsField,
  visible: CameraSettings[SettingsField] | undefined,
  requested: CameraSettings[SettingsField]
): string {
  if (phase === "visible") return "";
  if (phase === "draft") {
    return visible === undefined
      ? `Draft ${formatSettingValue(field, requested)} · visible setting unknown`
      : `Draft ${formatSettingValue(field, requested)} · visible ${formatSettingValue(field, visible)}`;
  }
  if (phase === "unknown") return "Visible setting unknown";
  if (phase === "applied") return "Awaiting exact frame";
  if (phase === "rejected") return "Rejected · restored to authoritative setting";
  return visible === undefined
    ? "Awaiting camera · visible setting unknown"
    : `Awaiting camera · visible ${formatSettingValue(field, visible)}`;
}

function renderReadout(
  readout: Record<"exposure" | "gain" | "treatment", HTMLElement>,
  state: ControlState
): void {
  const requested = newestRequestedSettings(state);
  const visible = state.settings.visible?.settings ?? null;
  const awaitingPresentation = state.settings.pending !== null || state.submitted !== null;
  readout.exposure.textContent = formatExposure(requested.exposureMs);
  readout.gain.textContent = `ISO ${requested.gain}`;
  readout.treatment.textContent = treatmentLabel(requested.treatment);
  readout.exposure.classList.toggle(
    "is-requested",
    visible === null ? awaitingPresentation : visible.exposureMs !== requested.exposureMs
  );
  readout.gain.classList.toggle(
    "is-requested",
    visible === null ? awaitingPresentation : visible.gain !== requested.gain
  );
  readout.treatment.classList.toggle(
    "is-requested",
    visible === null ? awaitingPresentation : visible.treatment !== requested.treatment
  );
  for (const value of Object.values(readout)) {
    value.classList.toggle("is-unknown", visible === null && !awaitingPresentation);
  }
}

function newestRequestedSettings(state: ControlState): CameraSettings {
  return state.submitted ?? state.settings.pending?.settings ?? state.settings.applied.settings;
}

function formatSettingValue(field: SettingsField, value: CameraSettings[SettingsField]): string {
  if (field === "exposureMs" && typeof value === "number") {
    return formatExposure(value);
  }
  if (field === "gain") return `ISO ${String(value)}`;
  if (field === "treatment") return treatmentLabel(value as CameraSettings["treatment"]);
  return String(value);
}

function formatExposure(value: number): string {
  return value >= 1_000 ? `${value / 1_000} s` : `${value} ms`;
}

function treatmentLabel(value: CameraSettings["treatment"]): string {
  return value === "monochrome" ? "B&W" : "Colour";
}

function indexRatio(choices: readonly number[], value: number): number {
  const index = choices.indexOf(value);
  return index < 0 || choices.length < 2 ? 0 : index / (choices.length - 1);
}

function describeDisabledControl(
  control: HTMLButtonElement | HTMLInputElement,
  disabled: boolean,
  description: string
): void {
  if (disabled) {
    control.title = description;
    control.setAttribute("aria-description", description);
  } else {
    control.removeAttribute("title");
    control.removeAttribute("aria-description");
  }
}

function settingElements(name: "exposure" | "gain" | "treatment"): SettingElements {
  const shell = requiredElement(`[data-setting-slider="${name}"]`);
  return {
    shell,
    semantics: requiredElement(`[data-setting-semantics="${name}"]`),
    visibleMarker: shell.querySelector<HTMLElement>("[data-setting-marker=\"visible\"]"),
    requestedMarker: shell.querySelector<HTMLElement>("[data-setting-marker=\"requested\"]")
  };
}

function feedStateToken(status: ReturnType<typeof viewerProjection>["status"]): string {
  return status.toLowerCase().replaceAll(" ", "-");
}

function operatorFeedDetail(
  status: ReturnType<typeof viewerProjection>["status"],
  detail: string,
  frameAgeMs: number | undefined
): string {
  if (status === "Live") {
    if (detail.includes("Exposure") || detail.includes("Applying")) return "Exposure in progress";
    if (detail.includes("identity")) return "Freshness unknown";
    return frameAgeMs !== undefined && frameAgeMs >= 5_000
      ? `Frame ${Math.round(frameAgeMs / 1_000)} s old`
      : "";
  }
  if (status === "Waiting for first image") return "Exposure in progress";
  if (status === "Reconnecting") return "Restoring video";
  if (status === "Stale") {
    return detail.includes("unknown") ? "Freshness unknown" : "Frame overdue";
  }
  return detail.includes("Recovering") ? "Camera recovering" : "Camera unavailable";
}

function operatorControlNotice(notice: string): string {
  if (notice.startsWith("Draft discarded")) return "Unapplied changes discarded";
  if (notice.startsWith("Settings rejected")) return "Settings rejected";
  return notice;
}

function snapshotPixelsAvailable(
  video: HTMLVideoElement,
  retainedFrame: HTMLCanvasElement
): boolean {
  return (!retainedFrame.hidden && retainedFrame.width > 0 && retainedFrame.height > 0) ||
    (video.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA &&
      video.videoWidth > 0 && video.videoHeight > 0);
}

async function saveVisibleSnapshot(
  video: HTMLVideoElement,
  retainedFrame: HTMLCanvasElement,
  viewer: ViewerState
): Promise<string | null> {
  const useRetained = !retainedFrame.hidden && retainedFrame.width > 0 && retainedFrame.height > 0;
  const width = useRetained ? retainedFrame.width : video.videoWidth;
  const height = useRetained ? retainedFrame.height : video.videoHeight;
  if (width === 0 || height === 0) throw new Error("no visible frame is available");
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (context === null) throw new Error("snapshot canvas is unavailable");
  context.drawImage(useRetained ? retainedFrame : video, 0, 0, width, height);
  const exactFrame = useRetained || (
    viewer.mediaConnection === "connected" &&
    !viewer.awaitingCurrentPresentation &&
    viewer.correlationLostAtUnixUs === null
  ) ? viewer.trustworthyFrame : null;
  const filename = snapshotFilename(exactFrame === null ? null : {
    exposureCompletedAtUnixUs: exactFrame.exposureCompletedAtUnixUs,
    sourceGeneration: exactFrame.sourceGeneration
  });
  const blob = await canvasBlob(canvas);
  const picker = (window as SnapshotWindow).showSaveFilePicker;
  if (window.isSecureContext && picker !== undefined) {
    const handle = await picker({
      suggestedName: filename,
      types: [{ description: "PNG image", accept: { "image/png": [".png"] } }]
    });
    const writable = await handle.createWritable();
    await writable.write(blob);
    await writable.close();
    return "Snapshot saved";
  }
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.download = filename;
  link.href = url;
  link.hidden = true;
  document.body.append(link);
  link.click();
  link.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 0);
  return "Snapshot download started";
}

interface SnapshotWindow extends Window {
  showSaveFilePicker?: (options: {
    suggestedName: string;
    types: Array<{ description: string; accept: Record<string, string[]> }>;
  }) => Promise<SnapshotFileHandle>;
}

interface SnapshotFileHandle {
  createWritable(): Promise<SnapshotWritable>;
}

interface SnapshotWritable {
  write(blob: Blob): Promise<void>;
  close(): Promise<void>;
}

function canvasBlob(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob === null) reject(new Error("snapshot encoding failed"));
      else resolve(blob);
    }, "image/png");
  });
}

function requiredInput(selector: string): HTMLInputElement {
  const element = document.querySelector<HTMLInputElement>(selector);
  if (element === null) throw new Error(`viewer shell is missing ${selector}`);
  return element;
}

function requiredVideo(selector: string): HTMLVideoElement {
  const element = document.querySelector<HTMLVideoElement>(selector);
  if (element === null) throw new Error(`viewer shell is missing ${selector}`);
  return element;
}

function requiredCanvas(selector: string): HTMLCanvasElement {
  const element = document.querySelector<HTMLCanvasElement>(selector);
  if (element === null) throw new Error(`viewer shell is missing ${selector}`);
  return element;
}

function requiredButton(selector: string): HTMLButtonElement {
  const element = document.querySelector<HTMLButtonElement>(selector);
  if (element === null) throw new Error(`viewer shell is missing ${selector}`);
  return element;
}

function requiredElement(selector: string): HTMLElement {
  const element = document.querySelector<HTMLElement>(selector);
  if (element === null) throw new Error(`viewer shell is missing ${selector}`);
  return element;
}

void boot();
