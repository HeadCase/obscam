import { deriveViewerState, deriveWhepUrl, parseRuntimeContract } from "./model.js";
import { startWhep } from "./whep.js";

async function boot(): Promise<void> {
  const status = requiredElement("[data-viewer-status]");
  const detail = requiredElement("[data-viewer-detail]");
  const unavailable = requiredElement("[data-viewer-unavailable]");
  const video = requiredVideo("[data-viewer-video]");

  try {
    const response = await fetch("/api/v1/runtime", {
      cache: "no-store",
      headers: { Accept: "application/json" }
    });
    if (!response.ok) {
      throw new Error(`runtime request failed with ${response.status}`);
    }
    const runtime = parseRuntimeContract(await response.json());
    const viewer = deriveViewerState(runtime);
    status.textContent = viewer.status;
    detail.textContent = unavailableDetail(runtime.components);
    video.addEventListener(
      "playing",
      () => {
        unavailable.hidden = true;
      },
      { once: true }
    );
    try {
      const session = await startWhep(video, deriveWhepUrl(runtime.media, window.location.href));
      window.addEventListener("pagehide", () => void session.close(), { once: true });
    } catch (error: unknown) {
      detail.textContent = "Media unavailable";
      console.error("ObsCam WHEP connection failed", error);
    }
  } catch (error: unknown) {
    status.textContent = "Unavailable";
    detail.textContent = "Runtime status unavailable";
    console.error("ObsCam viewer bootstrap failed", error);
  }
}

function requiredVideo(selector: string): HTMLVideoElement {
  const element = document.querySelector<HTMLVideoElement>(selector);
  if (element === null) {
    throw new Error(`viewer shell is missing ${selector}`);
  }
  return element;
}

function unavailableDetail(components: {
  capture: { state: "ready" | "unavailable" };
  encoder: { state: "ready" | "unavailable" };
  relay: { state: "ready" | "unavailable" };
}): string {
  const unavailable = [
    ["Capture", components.capture.state],
    ["Encoder", components.encoder.state],
    ["Relay", components.relay.state]
  ]
    .filter(([, state]) => state === "unavailable")
    .map(([name]) => name);
  return `${unavailable.join(" · ")} unavailable`;
}

function requiredElement(selector: string): HTMLElement {
  const element = document.querySelector<HTMLElement>(selector);
  if (element === null) {
    throw new Error(`viewer shell is missing ${selector}`);
  }
  return element;
}

void boot();
