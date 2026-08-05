import { initialControlState, reduceControl } from "./control.js";
import { initialPresentationState, reducePresentation } from "./presentation.js";
import { parseRuntimeComponents } from "./model.js";
const MIN_DELIVERY_ALLOWANCE_US = 250_000;
const MAX_DELIVERY_ALLOWANCE_US = 500_000;
const CORRELATION_GRACE_US = 1_000_000;
const MEDIA_LIVENESS_GRACE_US = 1_000_000;
/** Creates a new viewer fenced to one runtime epoch and optional tab credential. */
export function initialViewerState(runtimeEpoch, storedCredentials = null) {
    return {
        runtimeEpoch,
        lifecycle: null,
        control: initialControlState(storedCredentials, runtimeEpoch),
        presentation: initialPresentationState(runtimeEpoch),
        mediaConnection: "connecting",
        mediaConnectionGeneration: 0,
        lastMediaPresentedAtUnixUs: null,
        awaitingCurrentPresentation: true,
        visibility: "visible",
        trustworthyFrame: null,
        trustworthyPresentedAtUnixUs: null,
        correlationLostAtUnixUs: null,
        deliveryAllowanceUs: MIN_DELIVERY_ALLOWANCE_US,
        evidence: { p99DeliveryUs: null, response: null },
        nowUnixUs: 0
    };
}
/** Reduces one event without invoking browser APIs or mutating prior state. */
export function reduceViewer(state, event) {
    let next = state;
    let effects = [];
    let controlStorage = "none";
    let presentation = null;
    switch (event.type) {
        case "lifecycle": {
            const changedEpoch = event.facts.runtimeEpoch !== state.runtimeEpoch;
            const mediaRecovered = state.lifecycle?.recovery != null && event.facts.recovery === null;
            const sourceFloorAdvanced = state.lifecycle !== null &&
                event.facts.minimumSourceGeneration > state.lifecycle.minimumSourceGeneration;
            next = {
                ...state,
                runtimeEpoch: event.facts.runtimeEpoch,
                lifecycle: event.facts,
                control: changedEpoch
                    ? initialControlState(null, event.facts.runtimeEpoch)
                    : state.control,
                presentation: changedEpoch
                    ? initialPresentationState(event.facts.runtimeEpoch)
                    : state.presentation,
                lastMediaPresentedAtUnixUs: changedEpoch || mediaRecovered
                    ? null
                    : state.lastMediaPresentedAtUnixUs,
                correlationLostAtUnixUs: changedEpoch ? null : state.correlationLostAtUnixUs,
                awaitingCurrentPresentation: changedEpoch || sourceFloorAdvanced || event.facts.recovery !== null ||
                    state.awaitingCurrentPresentation
            };
            if (changedEpoch || mediaRecovered) {
                effects = ["reconnect_media"];
            }
            break;
        }
        case "control": {
            const transition = reduceControl(state.control, event.event);
            next = { ...state, control: transition.state };
            controlStorage = transition.storage;
            break;
        }
        case "mapping": {
            const transition = reducePresentation(state.presentation, {
                type: "mapping",
                mapping: event.mapping
            });
            const streamChanged = transition.state.streamEpoch > state.presentation.streamEpoch;
            if (transition.presented !== null && transition.presentedAtUnixUs !== null) {
                const exact = acceptExactPresentation(state, transition.state, transition.presented, transition.presentedAtUnixUs);
                next = exact.state;
                controlStorage = exact.controlStorage;
                presentation = {
                    mapping: transition.presented,
                    presentedAtUnixUs: transition.presentedAtUnixUs
                };
            }
            else {
                next = {
                    ...state,
                    presentation: transition.state,
                    awaitingCurrentPresentation: streamChanged || state.awaitingCurrentPresentation
                };
            }
            break;
        }
        case "media_connecting": {
            const connectionGeneration = event.connectionGeneration ?? state.mediaConnectionGeneration + 1;
            if (connectionGeneration <= state.mediaConnectionGeneration)
                break;
            const presentation = reducePresentation(state.presentation, {
                type: "reconnected",
                streamEpoch: state.presentation.streamEpoch
            }).state;
            next = {
                ...state,
                presentation,
                mediaConnection: "connecting",
                mediaConnectionGeneration: connectionGeneration,
                lastMediaPresentedAtUnixUs: null,
                awaitingCurrentPresentation: true,
                correlationLostAtUnixUs: null
            };
            break;
        }
        case "media_connected": {
            const connectionGeneration = event.connectionGeneration ?? state.mediaConnectionGeneration;
            if (connectionGeneration !== state.mediaConnectionGeneration)
                break;
            next = { ...state, mediaConnection: "connected" };
            break;
        }
        case "media_disconnected": {
            const connectionGeneration = event.connectionGeneration ?? state.mediaConnectionGeneration;
            if (connectionGeneration !== state.mediaConnectionGeneration)
                break;
            next = { ...state, mediaConnection: "disconnected", awaitingCurrentPresentation: true };
            break;
        }
        case "presented": {
            if (!acceptsMediaPresentation(state, event.mediaConnectionGeneration)) {
                break;
            }
            const mediaState = {
                ...state,
                lastMediaPresentedAtUnixUs: event.nowUnixUs,
                nowUnixUs: event.nowUnixUs
            };
            const transition = reducePresentation(mediaState.presentation, {
                type: "presented",
                ...(event.rtpTimestamp === undefined ? {} : { rtpTimestamp: event.rtpTimestamp }),
                nowUnixUs: event.nowUnixUs
            });
            if (transition.presented === null) {
                next = {
                    ...mediaState,
                    presentation: transition.state,
                    correlationLostAtUnixUs: state.correlationLostAtUnixUs ?? event.nowUnixUs,
                };
            }
            else {
                const exact = acceptExactPresentation(mediaState, transition.state, transition.presented, event.nowUnixUs);
                next = exact.state;
                controlStorage = exact.controlStorage;
                presentation = { mapping: transition.presented, presentedAtUnixUs: event.nowUnixUs };
            }
            break;
        }
        case "tick":
            if (state.visibility === "visible") {
                next = { ...state, nowUnixUs: event.nowUnixUs };
            }
            break;
        case "visibility":
            if (event.visibility === "visible" && state.visibility === "hidden") {
                next = {
                    ...state,
                    visibility: "visible",
                    mediaConnection: "connecting",
                    awaitingCurrentPresentation: true
                };
                effects = ["reconnect_media"];
            }
            else {
                next = { ...state, visibility: event.visibility };
            }
            break;
        case "evidence": {
            const allowance = event.p99DeliveryUs === null
                ? MIN_DELIVERY_ALLOWANCE_US
                : Math.min(MAX_DELIVERY_ALLOWANCE_US, Math.max(MIN_DELIVERY_ALLOWANCE_US, event.p99DeliveryUs));
            next = {
                ...state,
                deliveryAllowanceUs: allowance,
                evidence: { p99DeliveryUs: event.p99DeliveryUs, response: event.response }
            };
            break;
        }
    }
    return { state: next, effects, controlStorage, presentation };
}
function acceptExactPresentation(state, presentation, mapping, presentedAtUnixUs) {
    const currentSource = state.lifecycle !== null &&
        mapping.sourceGeneration >= state.lifecycle.minimumSourceGeneration;
    const advancesFrame = currentSource && (state.trustworthyFrame === null ||
        mapping.sourceGeneration >= state.trustworthyFrame.sourceGeneration);
    const control = currentSource
        ? reduceControl(state.control, {
            type: "visible",
            settingsGeneration: mapping.settingsGeneration
        })
        : { state: state.control, storage: "none" };
    return {
        state: {
            ...state,
            control: control.state,
            presentation,
            trustworthyFrame: advancesFrame ? mapping : state.trustworthyFrame,
            trustworthyPresentedAtUnixUs: advancesFrame
                ? presentedAtUnixUs
                : state.trustworthyPresentedAtUnixUs,
            awaitingCurrentPresentation: currentSource ? false : state.awaitingCurrentPresentation,
            correlationLostAtUnixUs: null,
            nowUnixUs: presentedAtUnixUs
        },
        controlStorage: control.storage
    };
}
/** Whether a frame observation belongs to the currently connected media session. */
export function acceptsMediaPresentation(state, mediaConnectionGeneration) {
    return mediaConnectionGeneration === state.mediaConnectionGeneration &&
        state.mediaConnection === "connected";
}
/** Projects state into primary media status while omitting unproven frame facts. */
export function viewerProjection(state) {
    const frame = state.trustworthyFrame;
    const lifecycle = state.lifecycle;
    const base = { controlConnection: state.control.connection };
    const failed = lifecycle === null ? null : failedComponent(lifecycle);
    const priorEpoch = frame !== null && frame.runtimeEpoch !== state.runtimeEpoch;
    const priorSource = frame !== null && lifecycle !== null &&
        frame.sourceGeneration < lifecycle.minimumSourceGeneration;
    const correlationUnknown = state.correlationLostAtUnixUs !== null &&
        state.nowUnixUs - state.correlationLostAtUnixUs > CORRELATION_GRACE_US;
    const mediaLive = state.mediaConnection === "connected" &&
        state.lastMediaPresentedAtUnixUs !== null &&
        state.nowUnixUs - state.lastMediaPresentedAtUnixUs <= MEDIA_LIVENESS_GRACE_US;
    if (failed !== null) {
        return frame === null
            ? { ...base, status: "Unavailable", detail: `Recovering ${failed}` }
            : { ...base, ...knownFrame(state, frame), status: "Stale", detail: `Recovering ${failed}` };
    }
    if (priorEpoch) {
        return {
            ...base,
            status: "Stale",
            detail: knownDetail(state, frame, "Frame from previous service epoch"),
            frameCompletedAtUnixUs: frame.exposureCompletedAtUnixUs,
            frameAgeMs: frameAgeMs(state, frame)
        };
    }
    if (priorSource) {
        if (state.control.settings.pending !== null && mediaLive) {
            return {
                ...base,
                ...knownFrame(state, frame),
                status: "Live",
                detail: knownDetail(state, frame, `Applying generation ${state.control.settings.pending.generation}`)
            };
        }
        return {
            ...base,
            ...knownFrame(state, frame),
            status: "Reconnecting",
            detail: knownDetail(state, frame, "Awaiting recovered frame")
        };
    }
    if (state.awaitingCurrentPresentation && frame !== null && !mediaLive) {
        return {
            ...base,
            ...knownFrame(state, frame),
            status: "Reconnecting",
            detail: knownDetail(state, frame, "Restoring video")
        };
    }
    if (state.control.settings.pending !== null) {
        const pendingCapture = lifecycle?.capture;
        if (pendingCapture !== null &&
            pendingCapture !== undefined &&
            captureDeadlinePassed(state, pendingCapture)) {
            return frame === null
                ? { ...base, status: "Unavailable", detail: "Expected exposure overdue" }
                : correlationUnknown
                    ? { ...base, status: "Stale", detail: "Frame freshness unknown" }
                    : { ...base, ...knownFrame(state, frame), status: "Stale", detail: "Expected frame overdue" };
        }
        if (mediaLive) {
            return frame === null || correlationUnknown
                ? { ...base, status: "Live", detail: "Frame identity pending" }
                : {
                    ...base,
                    ...knownFrame(state, frame),
                    status: "Live",
                    detail: knownDetail(state, frame, `Applying generation ${state.control.settings.pending.generation}`)
                };
        }
        return {
            ...base,
            ...(frame === null || correlationUnknown ? {} : knownFrame(state, frame)),
            status: "Waiting for first image",
            detail: correlationUnknown
                ? "Exposure in progress · frame freshness unknown"
                : frame === null
                    ? `Applying generation ${state.control.settings.pending.generation}`
                    : knownDetail(state, frame, `Applying generation ${state.control.settings.pending.generation}`)
        };
    }
    if (frame === null) {
        if (mediaLive) {
            return { ...base, status: "Live", detail: "Frame identity pending" };
        }
        if (lifecycle?.capture !== null && lifecycle?.capture !== undefined) {
            if (captureDeadlinePassed(state, lifecycle.capture)) {
                return { ...base, status: "Unavailable", detail: "Expected exposure overdue" };
            }
            return { ...base, status: "Waiting for first image", detail: "Waiting for first exposure" };
        }
        if (state.mediaConnection !== "disconnected") {
            return { ...base, status: "Reconnecting", detail: "Connecting to video" };
        }
        return { ...base, status: "Unavailable", detail: "No trustworthy frame" };
    }
    if (state.mediaConnection !== "connected") {
        return {
            ...base,
            ...knownFrame(state, frame),
            status: "Reconnecting",
            detail: knownDetail(state, frame, "Restoring video")
        };
    }
    const capture = lifecycle?.capture;
    if (capture !== null && capture !== undefined && capture.startedAtUnixUs > frame.exposureCompletedAtUnixUs) {
        const elapsed = Math.max(0, state.nowUnixUs - capture.startedAtUnixUs);
        const deadline = capture.exposureMs * 1_000 + state.deliveryAllowanceUs;
        if (elapsed > deadline) {
            return correlationUnknown
                ? { ...base, status: "Stale", detail: "Frame freshness unknown" }
                : { ...base, ...knownFrame(state, frame), status: "Stale", detail: "Expected frame overdue" };
        }
        if (capture.exposureMs * 1_000 > state.deliveryAllowanceUs ||
            elapsed >= state.deliveryAllowanceUs) {
            if (mediaLive) {
                return correlationUnknown
                    ? { ...base, status: "Live", detail: "Frame identity pending" }
                    : {
                        ...base,
                        ...knownFrame(state, frame),
                        status: "Live",
                        detail: knownDetail(state, frame, "Exposure in progress")
                    };
            }
            return correlationUnknown
                ? { ...base, status: "Stale", detail: "Frame freshness unknown" }
                : {
                    ...base,
                    ...knownFrame(state, frame),
                    status: "Stale",
                    detail: knownDetail(state, frame, "Video stopped during exposure")
                };
        }
    }
    if (correlationUnknown) {
        return mediaLive
            ? { ...base, status: "Live", detail: "Frame identity pending" }
            : { ...base, status: "Stale", detail: "Frame freshness unknown" };
    }
    return {
        ...base,
        ...knownFrame(state, frame),
        status: "Live",
        detail: knownDetail(state, frame, `Generation ${frame.sourceGeneration}`)
    };
}
/** Fails closed while parsing untrusted lifecycle messages from the WebSocket. */
export function parseLifecycleFacts(value) {
    if (!isRecord(value) ||
        value.schemaVersion !== 1 ||
        value.type !== "lifecycle" ||
        !isUuid(value.runtimeEpoch) ||
        !Number.isSafeInteger(value.minimumSourceGeneration) ||
        typeof value.minimumSourceGeneration !== "number" ||
        value.minimumSourceGeneration < 1 ||
        ![null, "capture", "encoder", "relay"].includes(value.recovery) ||
        !validCapture(value.capture)) {
        throw new Error("invalid lifecycle facts");
    }
    const components = parseRuntimeComponents(value.components);
    return {
        runtimeEpoch: value.runtimeEpoch,
        minimumSourceGeneration: value.minimumSourceGeneration,
        components,
        recovery: value.recovery,
        capture: value.capture
    };
}
function knownFrame(state, frame) {
    const visibleLatencyMs = state.trustworthyPresentedAtUnixUs === null
        ? null
        : Math.max(0, Math.floor((state.trustworthyPresentedAtUnixUs - frame.exposureCompletedAtUnixUs) / 1_000));
    return {
        sourceGeneration: frame.sourceGeneration,
        frameCompletedAtUnixUs: frame.exposureCompletedAtUnixUs,
        frameAgeMs: frameAgeMs(state, frame),
        ...(visibleLatencyMs === null ? {} : { visibleLatencyMs })
    };
}
function knownDetail(state, frame, prefix) {
    return `${prefix} · frame age ${formatAge(frameAgeMs(state, frame))}`;
}
function formatAge(ageMs) {
    return ageMs < 1_000 ? `${ageMs} ms` : `${(ageMs / 1_000).toFixed(1)} s`;
}
function frameAgeMs(state, frame) {
    return Math.max(0, Math.floor((state.nowUnixUs - frame.exposureCompletedAtUnixUs) / 1_000));
}
function captureDeadlinePassed(state, capture) {
    return state.nowUnixUs - capture.startedAtUnixUs >
        capture.exposureMs * 1_000 + state.deliveryAllowanceUs;
}
function failedComponent(facts) {
    if (facts.components.capture.state === "unavailable")
        return "capture";
    if (facts.components.encoder.state === "unavailable")
        return "encoder";
    return facts.recovery;
}
function validCapture(value) {
    return value === null || (isRecord(value) &&
        Number.isSafeInteger(value.settingsGeneration) &&
        typeof value.settingsGeneration === "number" &&
        value.settingsGeneration >= 0 &&
        Number.isInteger(value.exposureMs) &&
        typeof value.exposureMs === "number" &&
        value.exposureMs >= 10 &&
        value.exposureMs <= 30_000 &&
        Number.isSafeInteger(value.startedAtUnixUs) &&
        typeof value.startedAtUnixUs === "number" &&
        value.startedAtUnixUs >= 0);
}
function isUuid(value) {
    return typeof value === "string" &&
        /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu.test(value);
}
function isRecord(value) {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}
