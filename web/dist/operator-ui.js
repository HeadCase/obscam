export const CHROME_INACTIVITY_MS = 5_000;
export const FRAME_ARRIVAL_ACKNOWLEDGEMENT_MS = 300;
export const SETTING_REJECTION_ACKNOWLEDGEMENT_MS = 650;
export function initialOperatorUiState(nowMs) {
    return {
        chromeVisible: true,
        settingsOpen: false,
        lastActivityAtMs: nowMs,
        protections: [],
        frameArrivalUntilMs: null,
        settingRejectionUntilMs: null
    };
}
export function reduceOperatorUi(state, event) {
    switch (event.type) {
        case "activity":
            return reveal(state, event.nowMs);
        case "chrome_toggled":
            return state.chromeVisible
                ? { ...state, chromeVisible: false, settingsOpen: false, lastActivityAtMs: event.nowMs }
                : reveal(state, event.nowMs);
        case "tick": {
            const frameArrivalUntilMs = state.frameArrivalUntilMs !== null && event.nowMs >= state.frameArrivalUntilMs
                ? null
                : state.frameArrivalUntilMs;
            const settingRejectionUntilMs = state.settingRejectionUntilMs !== null && event.nowMs >= state.settingRejectionUntilMs
                ? null
                : state.settingRejectionUntilMs;
            if (state.chromeVisible &&
                state.protections.length === 0 &&
                event.nowMs - state.lastActivityAtMs >= CHROME_INACTIVITY_MS) {
                return {
                    ...state,
                    chromeVisible: false,
                    settingsOpen: false,
                    frameArrivalUntilMs,
                    settingRejectionUntilMs
                };
            }
            return frameArrivalUntilMs === state.frameArrivalUntilMs &&
                settingRejectionUntilMs === state.settingRejectionUntilMs
                ? state
                : { ...state, frameArrivalUntilMs, settingRejectionUntilMs };
        }
        case "protection_changed": {
            const protections = new Set(state.protections);
            if (event.active)
                protections.add(event.protection);
            else
                protections.delete(event.protection);
            return {
                ...reveal(state, event.nowMs),
                protections: [...protections]
            };
        }
        case "settings_toggled":
            return { ...reveal(state, event.nowMs), settingsOpen: !state.settingsOpen };
        case "settings_closed":
            return { ...reveal(state, event.nowMs), settingsOpen: false };
        case "authority_granted":
            return { ...reveal(state, event.nowMs), settingsOpen: true };
        case "authority_lost":
            return { ...reveal(state, event.nowMs), settingsOpen: false };
        case "exact_frame_presented":
            return {
                ...state,
                frameArrivalUntilMs: event.nowMs + FRAME_ARRIVAL_ACKNOWLEDGEMENT_MS
            };
        case "setting_rejected":
            return {
                ...state,
                settingRejectionUntilMs: event.nowMs + SETTING_REJECTION_ACKNOWLEDGEMENT_MS
            };
    }
}
export function exposureRailProgress(exposureMs, startedAtMs, nowMs) {
    if (exposureMs < 1_000)
        return null;
    return Math.min(1, Math.max(0, (nowMs - startedAtMs) / exposureMs));
}
export function initialExposureRailState() {
    return {
        captureStartedAtUnixUs: null,
        exposureMs: null,
        progress: null
    };
}
export function advanceExposureRail(state, sample) {
    if (sample === null || sample.exposureMs < 1_000)
        return initialExposureRailState();
    const progress = exposureRailProgress(sample.exposureMs, sample.startedAtUnixUs / 1_000, sample.nowUnixUs / 1_000);
    if (progress === null)
        return initialExposureRailState();
    const sameCapture = state.captureStartedAtUnixUs === sample.startedAtUnixUs &&
        state.exposureMs === sample.exposureMs;
    return {
        captureStartedAtUnixUs: sample.startedAtUnixUs,
        exposureMs: sample.exposureMs,
        progress: sameCapture && state.progress !== null
            ? Math.max(state.progress, progress)
            : progress
    };
}
export function settingPresentationPhase(facts) {
    if (facts.rejected)
        return "rejected";
    if (facts.drafted)
        return "draft";
    if (!facts.visibleKnown) {
        if (!facts.awaitingPresentation)
            return "unknown";
        return facts.applied ? "applied" : "requested";
    }
    if (facts.requestedDiffers && facts.awaitingPresentation) {
        return facts.applied ? "applied" : "requested";
    }
    return "visible";
}
function reveal(state, nowMs) {
    return { ...state, chromeVisible: true, lastActivityAtMs: nowMs };
}
