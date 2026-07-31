import { deriveViewerState, deriveWhepUrl, parseRuntimeContract } from "./model.js";
import { ControlClient } from "./control.js";
import { startWhep } from "./whep.js";
import { initialPresentationState, reducePresentation } from "./presentation.js";
import { ServiceQualityClient, downloadServiceQuality, serviceQualityText } from "./service-quality.js";
async function boot() {
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
    const exposureButtons = Array.from(document.querySelectorAll("[data-exposure-ms]"));
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
        const viewer = deriveViewerState(runtime);
        let latestSettings = { exposureMs: 500, gain: 100, treatment: "monochrome" };
        let presentation = initialPresentationState(runtime.runtimeEpoch);
        let qualityEvidence = null;
        let quality = null;
        let control;
        control = new ControlClient(runtime.runtimeEpoch, (state) => {
            if (state.connection === "disconnected") {
                presentation = reducePresentation(presentation, {
                    type: "reconnected",
                    streamEpoch: presentation.streamEpoch
                }).state;
            }
            latestSettings = state.settings.pending?.settings ?? state.settings.applied.settings;
            renderControl(state, takeControl, controlStatus, exposureButtons, gain, gainOutput, monochrome, colour);
        }, (mapping) => {
            presentation = reducePresentation(presentation, { type: "mapping", mapping }).state;
        });
        takeControl.addEventListener("click", () => control.toggleAuthority());
        for (const button of exposureButtons) {
            button.addEventListener("click", () => {
                control.setSettings({ ...latestSettings, exposureMs: Number(button.dataset.exposureMs) });
            });
        }
        gain.addEventListener("change", () => {
            control.setSettings({ ...latestSettings, gain: Number(gain.value) });
        });
        monochrome.addEventListener("click", () => {
            control.setSettings({ ...latestSettings, treatment: "monochrome" });
        });
        colour.addEventListener("click", () => {
            control.setSettings({ ...latestSettings, treatment: "colour" });
        });
        control.start();
        window.addEventListener("pagehide", () => control.close(), { once: true });
        status.textContent = viewer.status;
        detail.textContent = unavailableDetail(runtime.components);
        video.addEventListener("playing", () => {
            unavailable.hidden = true;
        }, { once: true });
        try {
            try {
                quality = await ServiceQualityClient.connect(runtime.runtimeEpoch, (evidence) => {
                    qualityEvidence = evidence;
                    qualityDetail.textContent = serviceQualityText(evidence);
                    qualityDownload.disabled = false;
                });
                qualityDownload.addEventListener("click", () => {
                    if (qualityEvidence !== null && quality !== null) {
                        downloadServiceQuality(qualityEvidence, quality.clientId);
                    }
                });
            }
            catch (error) {
                qualityDetail.textContent = "Quality evidence unavailable";
                console.error("ObsCam service-quality connection failed", error);
            }
            const session = await startWhep(video, deriveWhepUrl(runtime.media, window.location.href));
            presentation = reducePresentation(presentation, {
                type: "reconnected",
                streamEpoch: presentation.streamEpoch
            }).state;
            watchPresentedFrames(video, (metadata) => {
                const transition = reducePresentation(presentation, {
                    type: "presented",
                    ...(metadata.rtpTimestamp === undefined ? {} : { rtpTimestamp: metadata.rtpTimestamp }),
                    nowUnixUs: Date.now() * 1_000
                });
                presentation = transition.state;
                quality?.report({
                    correlation: transition.presented === null ? "unknown" : "exact",
                    streamEpoch: transition.presented?.streamEpoch ??
                        (presentation.streamEpoch === 0 ? null : presentation.streamEpoch),
                    ...(transition.presented === null || metadata.rtpTimestamp === undefined
                        ? {}
                        : { rtpTimestamp: metadata.rtpTimestamp }),
                    presentedFrames: metadata.presentedFrames,
                    visibility: document.visibilityState === "visible" ? "visible" : "hidden"
                });
                if (transition.presented !== null) {
                    control.markVisible(transition.presented.settingsGeneration);
                    status.textContent = "Visible";
                    detail.textContent = `Generation ${transition.presented.sourceGeneration}`;
                    serviceStatus.textContent = "Visible";
                    serviceDetail.textContent = `Generation ${transition.presented.sourceGeneration}`;
                }
                else {
                    status.textContent = "Unknown";
                    detail.textContent = "Frame correlation unavailable";
                    serviceStatus.textContent = "Unknown";
                    serviceDetail.textContent = "Frame correlation unavailable";
                }
            });
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
function watchPresentedFrames(video, presented) {
    const callback = (_now, metadata) => {
        presented(metadata);
        video.requestVideoFrameCallback(callback);
    };
    video.requestVideoFrameCallback(callback);
}
function renderControl(state, takeControl, controlStatus, exposureButtons, gain, gainOutput, monochrome, colour) {
    takeControl.disabled = state.connection !== "connected" || state.pendingIntent;
    takeControl.textContent = state.ownership === "you" ? "Release control" : "Take control";
    const settings = state.settings.pending?.settings ?? state.settings.applied.settings;
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
        state.connection === "disconnected"
            ? "Control reconnecting"
            : state.settings.pending !== null
                ? "Applying settings"
                : state.ownership === "you"
                    ? "You have control"
                    : state.ownership === "another_viewer"
                        ? "Another viewer has control"
                        : "No one has control";
}
function requiredInput(selector) {
    const element = document.querySelector(selector);
    if (element === null) {
        throw new Error(`viewer shell is missing ${selector}`);
    }
    return element;
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
