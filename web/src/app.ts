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
  downloadServiceQuality,
  fetchServiceQualityReport,
  serviceQualityText,
  type QualityObservation,
  type ServiceQualitySummary
} from "./service-quality.js";
import { QualityObservationSettler } from "./quality-settling.js";

const LIVENESS_TICK_MS = 100;
const INITIAL_RETRY_MS = 250;
const MAXIMUM_RETRY_MS = 5_000;
const STABLE_CONNECTION_MS = 5_000;
const QUALITY_SETTLING_MS = 1_100;

async function boot(): Promise<void> {
  const status = requiredElement("[data-viewer-status]");
  const detail = requiredElement("[data-viewer-detail]");
  const unavailable = requiredElement("[data-viewer-unavailable]");
  const serviceStatus = requiredElement("[data-service-status]");
  const serviceDetail = requiredElement("[data-service-detail]");
  const qualityDetail = requiredElement("[data-service-quality]");
  const qualityDownload = requiredButton("[data-service-quality-download]");
  const video = requiredVideo("[data-viewer-video]");
  const retainedFrame = requiredCanvas("[data-viewer-retained-frame]");
  const takeControl = requiredButton("[data-control=\"take-control\"]");
  const controlStatus = requiredElement("[data-control-status]");
  const exposureButtons = Array.from(
    document.querySelectorAll<HTMLButtonElement>("[data-exposure-ms]")
  );
  const gain = requiredInput("[data-gain]");
  const gainOutput = requiredElement("[data-gain-output]");
  const monochrome = requiredButton("[data-control=\"treatment-monochrome\"]");
  const colour = requiredButton("[data-control=\"treatment-colour\"]");
  const applySettings = requiredButton("[data-control=\"apply-settings\"]");
  const discardSettings = requiredButton("[data-control=\"discard-settings\"]");
  const settingEvidence = {
    exposure: requiredElement("[data-setting-evidence=\"exposure\"]"),
    gain: requiredElement("[data-setting-evidence=\"gain\"]"),
    treatment: requiredElement("[data-setting-evidence=\"treatment\"]")
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
    let quality: ServiceQualityClient | null = null;
    let qualityRuntimeEpoch: string | null = null;
    let qualityConnection: Promise<void> | null = null;
    let qualityMediaGeneration = 0;
    let qualitySync: Promise<void> = Promise.resolve();
    let mediaSession: WhepSession | null = null;
    let mediaAttemptCancellation: AbortController | null = null;
    let stopPresentedFrames: (() => void) | null = null;
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
      status.textContent = projection.status;
      detail.textContent = projection.detail;
      serviceStatus.textContent = projection.status;
      serviceDetail.textContent = projection.detail;
      unavailable.hidden = viewer.trustworthyFrame !== null;
      renderControl(
        viewer.control,
        takeControl,
        controlStatus,
        exposureButtons,
        gain,
        gainOutput,
        monochrome,
        colour,
        applySettings,
        discardSettings,
        settingEvidence
      );
    };

    const connectQuality = async (): Promise<void> => {
      while (qualityRuntimeEpoch !== viewer.runtimeEpoch) {
        if (qualityConnection === null) {
          const targetEpoch = viewer.runtimeEpoch;
          qualityConnection = ServiceQualityClient.connect(targetEpoch, (evidence) => {
            if (viewer.runtimeEpoch !== targetEpoch) return;
            qualityDetail.textContent = serviceQualityText(evidence);
            qualityDownload.disabled = false;
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
      qualityDetail.textContent = "Quality evidence unavailable";
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
        if (previous !== null) void previous.close();
        mediaReconnect.connected(connectionGeneration);
        dispatch({ type: "media_connected", connectionGeneration });
        stopPresentedFrames = watchPresentedFrames(video, (metadata) => {
          presentedFrame(connectionGeneration, metadata);
        });
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
      const transition = dispatch({ type: "control", event });
      return { state: transition.state.control, storage: transition.controlStorage };
    };

    const presentedFrame = (
      mediaConnectionGeneration: number,
      metadata: VideoFrameCallbackMetadata
    ): void => {
      const currentMedia = acceptsMediaPresentation(viewer, mediaConnectionGeneration);
      const nowUnixUs = currentServerUnixUs(quality, qualityRuntimeEpoch, viewer.runtimeEpoch);
      if (nowUnixUs !== null) {
        const transition = dispatch({
          type: "presented",
          mediaConnectionGeneration,
          ...(metadata.rtpTimestamp === undefined ? {} : { rtpTimestamp: metadata.rtpTimestamp }),
          nowUnixUs
        });
        if (transition.presentation !== null) {
          markSettingsTiming("browser-presented-exact", {
            settingsGeneration: transition.presentation.mapping.settingsGeneration,
            unixUs: transition.presentation.presentedAtUnixUs
          });
        }
      }
      const exact =
        currentMedia &&
        nowUnixUs !== null &&
        !viewer.awaitingCurrentPresentation &&
        viewer.correlationLostAtUnixUs === null &&
        viewer.trustworthyFrame !== null &&
        viewer.trustworthyFrame.streamEpoch === viewer.presentation.streamEpoch;
      if (exact && viewer.trustworthyFrame !== null) {
        const frame = viewer.trustworthyFrame;
        captureTrustworthyFrame(
          video,
          retainedFrame,
          `${frame.runtimeEpoch}:${frame.streamEpoch}:${frame.sourceGeneration}`
        );
        retainedFrame.hidden = true;
      }
      const observation: QualityObservation = {
        correlation: exact ? "exact" : "unknown",
        streamEpoch: exact
          ? viewer.trustworthyFrame?.streamEpoch ?? null
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
        markSettingsTiming("exposure-completed", {
          settingsGeneration: mapping.settingsGeneration,
          unixUs: mapping.exposureCompletedAtUnixUs
        });
        markSettingsTiming("encode-published", {
          settingsGeneration: mapping.settingsGeneration,
          unixUs: mapping.submittedAtUnixUs
        });
        const transition = dispatch({ type: "mapping", mapping });
        if (transition.presentation !== null) {
          markSettingsTiming("browser-presented-exact", {
            settingsGeneration: transition.presentation.mapping.settingsGeneration,
            unixUs: transition.presentation.presentedAtUnixUs
          });
          qualitySettler.promote(
            transition.presentation.mapping.rtpTimestamp,
            transition.presentation.mapping.streamEpoch
          );
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
    for (const button of exposureButtons) {
      button.addEventListener("click", () => {
        markSettingsTiming("browser-action", { field: "exposure" });
        dispatch({ type: "control", event: { type: "draft_changed", settings: {
          ...selectedSettings(viewer.control),
          exposureMs: Number(button.dataset.exposureMs)
        } } });
      });
    }
    gain.addEventListener("input", () => {
      markSettingsTiming("browser-action", { field: "gain" });
      dispatch({ type: "control", event: { type: "draft_changed", settings: {
        ...selectedSettings(viewer.control),
        gain: Number(gain.value)
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

    render();
    control.start();
    qualityDownload.addEventListener("click", () => {
      if (quality === null) return;
      const clientId = quality.clientId;
      qualityDownload.disabled = true;
      void fetchServiceQualityReport(clientId)
        .then((evidence) => downloadServiceQuality(evidence, clientId))
        .catch(qualityFailed)
        .finally(() => {
          qualityDownload.disabled = false;
        });
    });

    document.addEventListener("visibilitychange", () => {
      const foreground = document.visibilityState === "visible" && viewer.visibility === "hidden";
      dispatch({
        type: "visibility",
        visibility: document.visibilityState === "visible" ? "visible" : "hidden"
      });
      if (foreground && viewer.control.connection === "disconnected") control.retryNow();
    });
    window.addEventListener("online", () => {
      mediaReconnect.retryNow();
      control.retryNow();
    });
    const tick = window.setInterval(() => {
      const nowUnixUs = currentServerUnixUs(quality, qualityRuntimeEpoch, viewer.runtimeEpoch);
      if (nowUnixUs !== null) {
        dispatch({ type: "tick", nowUnixUs });
      }
    }, LIVENESS_TICK_MS);
    window.addEventListener("pagehide", () => {
      window.clearInterval(tick);
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
  takeControl: HTMLButtonElement,
  controlStatus: HTMLElement,
  exposureButtons: HTMLButtonElement[],
  gain: HTMLInputElement,
  gainOutput: HTMLElement,
  monochrome: HTMLButtonElement,
  colour: HTMLButtonElement,
  applySettings: HTMLButtonElement,
  discardSettings: HTMLButtonElement,
  settingEvidence: Record<"exposure" | "gain" | "treatment", HTMLElement>
): void {
  takeControl.disabled = state.connection !== "connected" || state.pendingIntent;
  takeControl.textContent = state.ownership === "you" ? "Release control" : "Take control";
  const settings = selectedSettings(state);
  const settingsDisabled = !state.mayMutate;
  const disabledDescription = "Take control to change settings";
  for (const button of exposureButtons) {
    button.disabled = settingsDisabled;
    button.setAttribute("aria-pressed", String(Number(button.dataset.exposureMs) === settings.exposureMs));
    describeDisabledControl(button, settingsDisabled, disabledDescription);
  }
  gain.disabled = settingsDisabled;
  describeDisabledControl(gain, settingsDisabled, disabledDescription);
  gain.value = String(settings.gain);
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
  renderSettingEvidence(settingEvidence.exposure, "exposureMs", state);
  renderSettingEvidence(settingEvidence.gain, "gain", state);
  renderSettingEvidence(settingEvidence.treatment, "treatment", state);
  controlStatus.textContent = state.notice ?? (
    state.connection === "disconnected"
      ? state.retryDelayMs === null
        ? "Control reconnecting now"
        : `Control reconnecting in ${formatRetryDelay(state.retryDelayMs)}` :
      state.settings.pending !== null ? "Requested settings" :
        state.draft !== null ? "Unsaved changes" :
        state.ownership === "you" ? "You have control" :
          state.ownership === "another_viewer" ? "Another viewer has control" : "No one has control"
  );
}

type SettingsField = keyof CameraSettings;

function renderSettingEvidence(
  container: HTMLElement,
  field: SettingsField,
  state: ControlState
): void {
  const entries: Array<{ label: string; kind: string; settings: CameraSettings }> = [];
  if (state.settings.visible !== null) {
    entries.push({ label: "Visible", kind: "visible", settings: state.settings.visible.settings });
  }
  if (state.settings.pending !== null) {
    const applied = state.settings.applied.generation >= state.settings.pending.generation;
    entries.push({
      label: applied ? "Requested · awaiting visibility" : "Requested · awaiting camera",
      kind: "requested",
      settings: state.settings.pending.settings
    });
  }
  if (state.submitted !== null) {
    entries.push({ label: "Requested · awaiting acceptance", kind: "submitted", settings: state.submitted });
  }
  if (state.draft !== null) {
    entries.push({ label: "Draft", kind: "draft", settings: state.draft });
  }
  if (state.rejected !== null) {
    entries.push({ label: "Rejected", kind: "rejected", settings: state.rejected });
  }
  container.replaceChildren(...entries.map(({ label, kind, settings }) => {
    const marker = document.createElement("span");
    marker.className = `setting-evidence__marker setting-evidence__marker--${kind}`;
    marker.dataset.settingState = kind;
    marker.textContent = `${label}: ${formatSettingValue(field, settings[field])}`;
    return marker;
  }));
}

function formatSettingValue(field: SettingsField, value: CameraSettings[SettingsField]): string {
  if (field === "exposureMs" && typeof value === "number") {
    return value >= 1_000 ? `${value / 1_000} s` : `${value} ms`;
  }
  return String(value);
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

function formatRetryDelay(delayMs: number): string {
  return delayMs < 1_000 ? `${delayMs} ms` : `${(delayMs / 1_000).toFixed(1)} s`;
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
