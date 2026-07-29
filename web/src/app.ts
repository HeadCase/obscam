import { deriveViewerState, parseRuntimeContract } from "./model.js";

async function boot(): Promise<void> {
  const status = requiredElement("[data-viewer-status]");
  const detail = requiredElement("[data-viewer-detail]");

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
  } catch (error: unknown) {
    status.textContent = "Unavailable";
    detail.textContent = "Runtime status unavailable";
    console.error("ObsCam viewer bootstrap failed", error);
  }
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
