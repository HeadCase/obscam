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
  serviceQualityText,
  type ServiceQualityResponse
} from "./service-quality.js";

const LIVENESS_TICK_MS = 100;

async function boot(): Promise<void> {
  const status = requiredElement("[data-viewer-status]");
  const detail = requiredElement("[data-viewer-detail]");
  const unavailable = requiredElement("[data-viewer-unavailable]");
  const serviceStatus = requiredElement("[data-service-status]");
  const serviceDetail = requiredElement("[data-service-detail]");
  const qualityDetail = requiredElement("[data-service-quality]");
  const qualityDownload = requiredButton("[data-service-quality-download]");
  const video = requiredVideo("[data-viewer-video]");
  const takeControl = requiredButton("[data-control=\"take-control\"]");
  const controlStatus = requiredElement("[data-control-status]");
  const exposureButtons = Array.from(
    document.querySelectorAll<HTMLButtonElement>("[data-exposure-ms]")
  );
  const gain = requiredInput("[data-gain]");
  const gainOutput = requiredElement("[data-gain-output]");
  const monochrome = requiredButton("[data-control=\"treatment-monochrome\"]");
  const colour = requiredButton("[data-control=\"treatment-colour\"]");

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
    let mediaSession: WhepSession | null = null;
    let stopPresentedFrames: (() => void) | null = null;
    let mediaAttempt = 0;

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
        colour
      );
    };

    const connectQuality = async (): Promise<void> => {
      const targetEpoch = viewer.runtimeEpoch;
      const client = await ServiceQualityClient.connect(targetEpoch, (evidence) => {
        if (viewer.runtimeEpoch !== targetEpoch) return;
        qualityDetail.textContent = serviceQualityText(evidence);
        qualityDownload.disabled = false;
        dispatch({
          type: "evidence",
          p99DeliveryUs: currentDeliveryP99Us(evidence),
          response: evidence
        });
      });
      if (viewer.runtimeEpoch === targetEpoch) {
        quality = client;
        qualityRuntimeEpoch = targetEpoch;
      }
    };

    const connectMedia = async (reconnection: boolean): Promise<void> => {
      const attempt = ++mediaAttempt;
      const connectionGeneration = dispatch({ type: "media_connecting" }).state
        .mediaConnectionGeneration;
      stopPresentedFrames?.();
      stopPresentedFrames = null;
      const previous = mediaSession;
      mediaSession = null;
      if (previous !== null) {
        await previous.close();
      }
      try {
        if (qualityRuntimeEpoch !== viewer.runtimeEpoch) {
          await connectQuality();
        } else if (reconnection && quality !== null) {
          await quality.reconnect();
        }
      } catch (error: unknown) {
        qualityDetail.textContent = "Quality evidence unavailable";
        console.error("ObsCam service-quality connection failed", error);
      }
      try {
        const session = await startWhep(video, deriveWhepUrl(runtime.media, window.location.href));
        if (attempt !== mediaAttempt) {
          await session.close();
          return;
        }
        mediaSession = session;
        dispatch({ type: "media_connected" });
        stopPresentedFrames = watchPresentedFrames(video, (metadata) => {
          presentedFrame(connectionGeneration, metadata);
        });
      } catch (error: unknown) {
        if (attempt === mediaAttempt) {
          dispatch({ type: "media_disconnected" });
          console.error("ObsCam WHEP connection failed", error);
        }
      }
    };

    const dispatch = (event: ViewerEvent): ReturnType<typeof reduceViewer> => {
      const transition = reduceViewer(viewer, event);
      viewer = transition.state;
      render();
      if (transition.effects.includes("reconnect_media")) {
        void connectMedia(true);
      }
      return transition;
    };

    const controlTransition = (event: ControlEvent): ControlTransition => {
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
        dispatch({
          type: "presented",
          mediaConnectionGeneration,
          ...(metadata.rtpTimestamp === undefined ? {} : { rtpTimestamp: metadata.rtpTimestamp }),
          nowUnixUs
        });
      }
      const exact =
        currentMedia &&
        nowUnixUs !== null &&
        !viewer.awaitingCurrentPresentation &&
        viewer.correlationLostAtUnixUs === null &&
        viewer.trustworthyFrame !== null;
      quality?.report({
        correlation: exact ? "exact" : "unknown",
        streamEpoch:
          exact ? viewer.trustworthyFrame?.streamEpoch ?? null :
            viewer.presentation.streamEpoch === 0 ? null : viewer.presentation.streamEpoch,
        ...(exact && metadata.rtpTimestamp !== undefined
          ? { rtpTimestamp: metadata.rtpTimestamp }
          : {}),
        presentedFrames: metadata.presentedFrames,
        visibility: document.visibilityState === "visible" ? "visible" : "hidden"
      });
    };
    const control = new ControlClient(
      () => viewer.runtimeEpoch,
      () => viewer.control,
      controlTransition,
      (mapping) => dispatch({ type: "mapping", mapping }),
      (value) => dispatch({ type: "lifecycle", facts: parseLifecycleFacts(value) })
    );

    takeControl.addEventListener("click", () => control.toggleAuthority());
    for (const button of exposureButtons) {
      button.addEventListener("click", () => {
        control.setSettings({
          ...selectedSettings(viewer.control),
          exposureMs: Number(button.dataset.exposureMs)
        });
      });
    }
    gain.addEventListener("change", () => {
      control.setSettings({ ...selectedSettings(viewer.control), gain: Number(gain.value) });
    });
    monochrome.addEventListener("click", () => {
      control.setSettings({ ...selectedSettings(viewer.control), treatment: "monochrome" });
    });
    colour.addEventListener("click", () => {
      control.setSettings({ ...selectedSettings(viewer.control), treatment: "colour" });
    });

    render();
    control.start();
    try {
      await connectQuality();
      qualityDownload.addEventListener("click", () => {
        if (viewer.evidence.response !== null && quality !== null) {
          downloadServiceQuality(viewer.evidence.response, quality.clientId);
        }
      });
    } catch (error: unknown) {
      qualityDetail.textContent = "Quality evidence unavailable";
      console.error("ObsCam service-quality connection failed", error);
    }

    document.addEventListener("visibilitychange", () => {
      dispatch({
        type: "visibility",
        visibility: document.visibilityState === "visible" ? "visible" : "hidden"
      });
    });
    const tick = window.setInterval(() => {
      const nowUnixUs = currentServerUnixUs(quality, qualityRuntimeEpoch, viewer.runtimeEpoch);
      if (nowUnixUs !== null) {
        dispatch({ type: "tick", nowUnixUs });
      }
    }, LIVENESS_TICK_MS);
    window.addEventListener("pagehide", () => {
      window.clearInterval(tick);
      mediaAttempt += 1;
      control.close();
      stopPresentedFrames?.();
      void mediaSession?.close();
    }, { once: true });
    await connectMedia(false);
  } catch (error: unknown) {
    status.textContent = "Unavailable";
    detail.textContent = "Runtime status unavailable";
    console.error("ObsCam viewer bootstrap failed", error);
  }
}

function currentServerUnixUs(
  quality: ServiceQualityClient | null,
  qualityRuntimeEpoch: string | null,
  runtimeEpoch: string
): number | null {
  return quality !== null && qualityRuntimeEpoch === runtimeEpoch ? quality.nowUnixUs() : null;
}

function currentDeliveryP99Us(response: ServiceQualityResponse): number | null {
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
  return state.settings.pending?.settings ?? state.settings.applied.settings;
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

function renderControl(
  state: ControlState,
  takeControl: HTMLButtonElement,
  controlStatus: HTMLElement,
  exposureButtons: HTMLButtonElement[],
  gain: HTMLInputElement,
  gainOutput: HTMLElement,
  monochrome: HTMLButtonElement,
  colour: HTMLButtonElement
): void {
  takeControl.disabled = state.connection !== "connected" || state.pendingIntent;
  takeControl.textContent = state.ownership === "you" ? "Release control" : "Take control";
  const settings = selectedSettings(state);
  const settingsDisabled = !state.mayMutate || state.pendingIntent;
  for (const button of exposureButtons) {
    button.disabled = settingsDisabled;
    button.setAttribute("aria-pressed", String(Number(button.dataset.exposureMs) === settings.exposureMs));
  }
  gain.disabled = settingsDisabled;
  gain.value = String(settings.gain);
  gainOutput.textContent = String(settings.gain);
  monochrome.disabled = settingsDisabled;
  colour.disabled = settingsDisabled;
  monochrome.setAttribute("aria-pressed", String(settings.treatment === "monochrome"));
  colour.setAttribute("aria-pressed", String(settings.treatment === "colour"));
  controlStatus.textContent =
    state.connection === "disconnected" ? "Control reconnecting" :
      state.settings.pending !== null ? "Applying settings" :
        state.ownership === "you" ? "You have control" :
          state.ownership === "another_viewer" ? "Another viewer has control" : "No one has control";
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
