import { deriveViewerState, deriveWhepUrl, parseRuntimeContract } from "./model.js";
import { ControlClient } from "./control.js";
import { startWhep } from "./whep.js";
async function boot() {
    const status = requiredElement("[data-viewer-status]");
    const detail = requiredElement("[data-viewer-detail]");
    const unavailable = requiredElement("[data-viewer-unavailable]");
    const video = requiredVideo("[data-viewer-video]");
    const takeControl = requiredButton("[data-control=\"take-control\"]");
    const controlStatus = requiredElement("[data-control-status]");
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
        const control = new ControlClient(runtime.runtimeEpoch, (state) => renderControl(state, takeControl, controlStatus));
        takeControl.addEventListener("click", () => control.toggleAuthority());
        control.start();
        window.addEventListener("pagehide", () => control.close(), { once: true });
        status.textContent = viewer.status;
        detail.textContent = unavailableDetail(runtime.components);
        video.addEventListener("playing", () => {
            unavailable.hidden = true;
        }, { once: true });
        try {
            const session = await startWhep(video, deriveWhepUrl(runtime.media, window.location.href));
            window.addEventListener("pagehide", () => void session.close(), { once: true });
        }
        catch (error) {
            detail.textContent = "Media unavailable";
            console.error("ObsCam WHEP connection failed", error);
        }
    }
    catch (error) {
        status.textContent = "Unavailable";
        detail.textContent = "Runtime status unavailable";
        console.error("ObsCam viewer bootstrap failed", error);
    }
}
function renderControl(state, takeControl, controlStatus) {
    takeControl.disabled = state.connection !== "connected" || state.pendingIntent;
    takeControl.textContent = state.ownership === "you" ? "Release control" : "Take control";
    controlStatus.textContent =
        state.connection === "disconnected"
            ? "Control reconnecting"
            : state.ownership === "you"
                ? "You have control"
                : state.ownership === "another_viewer"
                    ? "Another viewer has control"
                    : "No one has control";
}
function requiredVideo(selector) {
    const element = document.querySelector(selector);
    if (element === null) {
        throw new Error(`viewer shell is missing ${selector}`);
    }
    return element;
}
function requiredButton(selector) {
    const element = document.querySelector(selector);
    if (element === null) {
        throw new Error(`viewer shell is missing ${selector}`);
    }
    return element;
}
function unavailableDetail(components) {
    const unavailable = [
        ["Capture", components.capture.state],
        ["Encoder", components.encoder.state],
        ["Relay", components.relay.state]
    ]
        .filter(([, state]) => state === "unavailable")
        .map(([name]) => name);
    return `${unavailable.join(" · ")} unavailable`;
}
function requiredElement(selector) {
    const element = document.querySelector(selector);
    if (element === null) {
        throw new Error(`viewer shell is missing ${selector}`);
    }
    return element;
}
void boot();
