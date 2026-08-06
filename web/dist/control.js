import { parseFrameMapping } from "./presentation.js";
import { ReconnectLoop } from "./reconnect.js";
const STORAGE_KEY = "obscam.control.v1";
const LEASE_DURATION_MS = 5_000;
const RENEWAL_INTERVAL_MS = 2_000;
const INITIAL_RECONNECT_DELAY_MS = 250;
const MAXIMUM_RECONNECT_DELAY_MS = 5_000;
const STABLE_CONNECTION_MS = 5_000;
const EXPOSURE_CHOICES_MS = new Set([
    50, 100, 200, 300, 500, 1_000, 2_000, 5_000, 10_000, 15_000, 20_000, 30_000
]);
export function initialControlState(stored, runtimeEpoch = stored?.runtimeEpoch ?? "") {
    const credentials = stored !== null && stored.runtimeEpoch === runtimeEpoch && validCredentials(stored)
        ? stored
        : null;
    return withDerivedMutationPermission({
        connection: "disconnected",
        connectionGeneration: 0,
        retryDelayMs: null,
        ownership: "no_one",
        generation: credentials?.generation ?? 0,
        credentials,
        credentialsValidated: false,
        pendingIntent: false,
        draft: null,
        submitted: null,
        rejected: null,
        notice: null,
        settings: {
            applied: {
                generation: 0,
                settings: { exposureMs: 500, gain: 100, treatment: "monochrome" }
            },
            pending: null,
            visible: null,
            presentedGeneration: null
        }
    });
}
export function reduceControl(state, event) {
    let storage = "none";
    let next = state;
    switch (event.type) {
        case "connecting":
            if (event.connectionGeneration <= state.connectionGeneration)
                return { state, storage };
            next = {
                ...state,
                connection: "disconnected",
                connectionGeneration: event.connectionGeneration,
                retryDelayMs: null,
                ownership: "no_one",
                credentialsValidated: false,
                pendingIntent: false,
                draft: null,
                submitted: null,
                notice: state.draft !== null || state.submitted !== null
                    ? "Draft discarded: control reconnecting"
                    : state.notice
            };
            break;
        case "connected": {
            if (event.connectionGeneration !== state.connectionGeneration)
                return { state, storage };
            next = { ...state, connection: "connected", retryDelayMs: null };
            break;
        }
        case "disconnected": {
            if (event.connectionGeneration !== state.connectionGeneration)
                return { state, storage };
            next = {
                ...state,
                connection: "disconnected",
                retryDelayMs: null,
                ownership: "no_one",
                credentialsValidated: false,
                pendingIntent: false,
                draft: null,
                submitted: null,
                notice: state.draft !== null || state.submitted !== null
                    ? "Draft discarded: control disconnected"
                    : state.notice
            };
            break;
        }
        case "retry_scheduled":
            if (event.connectionGeneration !== state.connectionGeneration)
                return { state, storage };
            next = { ...state, retryDelayMs: event.retryDelayMs };
            break;
        case "granted":
            next = {
                ...state,
                ownership: "you",
                generation: event.credentials.generation,
                credentials: event.credentials,
                credentialsValidated: true,
                pendingIntent: false,
                notice: null
            };
            storage = "save";
            break;
        case "authority": {
            if (event.generation < state.generation) {
                return { state, storage };
            }
            const sameCredentials = state.credentials?.generation === event.generation;
            const ours = event.state === "held" && sameCredentials && state.credentialsValidated;
            const retainCredentials = event.state === "held" && sameCredentials;
            const displaced = state.credentials !== null && !retainCredentials;
            next = {
                ...state,
                ownership: event.state === "unheld" ? "no_one" : ours ? "you" : "another_viewer",
                generation: event.generation,
                credentials: retainCredentials ? state.credentials : null,
                credentialsValidated: ours,
                pendingIntent: false,
                draft: retainCredentials ? state.draft : null,
                submitted: retainCredentials ? state.submitted : null,
                notice: !retainCredentials && (state.draft !== null || state.submitted !== null)
                    ? "Draft discarded: control lost"
                    : state.notice
            };
            storage = displaced ? "remove" : "none";
            break;
        }
        case "resumed":
        case "renewed":
            if (state.credentials?.generation === event.generation) {
                next = {
                    ...state,
                    ownership: "you",
                    generation: event.generation,
                    credentialsValidated: true
                };
            }
            break;
        case "released":
            if (event.generation >= state.generation) {
                next = {
                    ...state,
                    ownership: "no_one",
                    generation: event.generation,
                    credentials: null,
                    credentialsValidated: false,
                    pendingIntent: false,
                    draft: null,
                    submitted: null,
                    notice: state.draft !== null || state.submitted !== null
                        ? "Draft discarded: control released"
                        : state.notice
                };
                storage = state.credentials === null ? "none" : "remove";
            }
            break;
        case "settings":
            next = {
                ...state,
                settings: {
                    ...event.state,
                    visible: state.settings.visible,
                    presentedGeneration: event.state.pending?.generation === state.settings.presentedGeneration
                        ? state.settings.presentedGeneration
                        : null
                }
            };
            break;
        case "accepted":
            next = {
                ...state,
                pendingIntent: false,
                submitted: null,
                rejected: null,
                notice: null,
                settings: {
                    ...state.settings,
                    pending: { generation: event.targetGeneration, settings: event.settings },
                    presentedGeneration: null
                }
            };
            break;
        case "applied":
            next = {
                ...state,
                pendingIntent: false,
                submitted: null,
                settings: {
                    applied: { generation: event.settingsGeneration, settings: event.settings },
                    pending: state.settings.presentedGeneration === event.settingsGeneration
                        ? null
                        : state.settings.pending !== null &&
                            state.settings.pending.generation >= event.settingsGeneration
                            ? state.settings.pending
                            : null,
                    visible: state.settings.presentedGeneration === event.settingsGeneration
                        ? { generation: event.settingsGeneration, settings: event.settings }
                        : state.settings.visible,
                    presentedGeneration: null
                }
            };
            break;
        case "visible":
            if (state.settings.pending?.generation === event.settingsGeneration ||
                state.settings.applied.generation === event.settingsGeneration) {
                const appliedIsVisible = state.settings.applied.generation === event.settingsGeneration;
                next = {
                    ...state,
                    settings: {
                        ...state.settings,
                        pending: appliedIsVisible
                            ? null
                            : state.settings.pending,
                        visible: appliedIsVisible
                            ? state.settings.applied
                            : state.settings.visible,
                        presentedGeneration: appliedIsVisible
                            ? null
                            : event.settingsGeneration
                    }
                };
            }
            break;
        case "failed":
            {
                const failedSettings = state.settings.pending?.generation === event.targetGeneration
                    ? state.settings.pending.settings
                    : null;
                next = {
                    ...state,
                    pendingIntent: false,
                    submitted: null,
                    rejected: failedSettings,
                    notice: failedSettings === null ? state.notice : "Settings rejected: camera recovery",
                    settings: {
                        ...state.settings,
                        pending: state.settings.pending?.generation === event.targetGeneration
                            ? null
                            : state.settings.pending,
                        presentedGeneration: state.settings.presentedGeneration === event.targetGeneration
                            ? null
                            : state.settings.presentedGeneration
                    }
                };
                break;
            }
        case "rejected": {
            const authorityLost = event.reason === undefined || ["not_holder", "expired"].includes(event.reason);
            next = authorityLost
                ? {
                    ...state,
                    ownership: state.ownership === "another_viewer" ? "another_viewer" : "no_one",
                    credentials: null,
                    credentialsValidated: false,
                    pendingIntent: false,
                    draft: null,
                    submitted: null,
                    notice: state.draft !== null || state.submitted !== null
                        ? "Draft discarded: control lost"
                        : state.notice
                }
                : {
                    ...state,
                    pendingIntent: false,
                    draft: null,
                    submitted: null,
                    rejected: state.submitted ?? state.draft,
                    notice: `Settings rejected: ${rejectionDescription(event.reason)}`
                };
            storage = authorityLost && state.credentials !== null ? "remove" : "none";
            break;
        }
        case "intent_queued":
            next = {
                ...state,
                pendingIntent: state.mayMutate,
                ...(event.settings === undefined
                    ? {}
                    : {
                        draft: null,
                        submitted: event.settings,
                        rejected: null,
                        notice: "Settings submitted"
                    })
            };
            break;
        case "draft_changed":
            if (state.mayMutate) {
                next = {
                    ...state,
                    rejected: null,
                    notice: null,
                    draft: sameCameraSettings(event.settings, state.submitted ?? state.settings.pending?.settings ?? state.settings.applied.settings)
                        ? null
                        : event.settings
                };
            }
            break;
        case "draft_discarded":
            next = { ...state, draft: null, notice: "Draft discarded" };
            break;
    }
    return { state: withDerivedMutationPermission(next), storage };
}
export function parseControlMessage(value) {
    if (!isRecord(value) || value.schemaVersion !== 1 || typeof value.type !== "string") {
        throw new Error("unsupported control message");
    }
    if (value.type === "authority") {
        if ((value.state === "held" || value.state === "unheld") && validGeneration(value.generation, true)) {
            return { type: "authority", state: value.state, generation: value.generation };
        }
    }
    else if (value.type === "granted") {
        if (validGeneration(value.generation) &&
            validSecret(value.secret) &&
            value.leaseDurationMs === LEASE_DURATION_MS) {
            return {
                type: "granted",
                generation: value.generation,
                secret: value.secret,
                leaseDurationMs: value.leaseDurationMs
            };
        }
    }
    else if (value.type === "renewed" || value.type === "resumed") {
        if (validGeneration(value.generation) && value.leaseDurationMs === LEASE_DURATION_MS) {
            return {
                type: value.type,
                generation: value.generation,
                leaseDurationMs: value.leaseDurationMs
            };
        }
    }
    else if (value.type === "released") {
        if (validGeneration(value.generation)) {
            return { type: "released", generation: value.generation };
        }
    }
    else if (value.type === "settings") {
        const state = parseSettingsState(value.state);
        if (state !== null) {
            return { type: "settings", state };
        }
    }
    else if (value.type === "accepted") {
        const settings = parseCameraSettings(value.settings);
        if (validGeneration(value.targetGeneration) && settings !== null) {
            return { type: "accepted", targetGeneration: value.targetGeneration, settings };
        }
    }
    else if (value.type === "applied") {
        const settings = parseCameraSettings(value.settings);
        if (validGeneration(value.settingsGeneration, true) && settings !== null) {
            return { type: "applied", settingsGeneration: value.settingsGeneration, settings };
        }
    }
    else if (value.type === "failed") {
        if (validGeneration(value.targetGeneration) &&
            (value.reason === "superseded" || value.reason === "recovery")) {
            return { type: "failed", targetGeneration: value.targetGeneration, reason: value.reason };
        }
    }
    else if (value.type === "rejected") {
        if (isRejectionReason(value.reason)) {
            return { type: "rejected", reason: value.reason };
        }
    }
    throw new Error("invalid control message");
}
/** Owns the browser effects around the pure control reducer. */
export class ControlClient {
    runtimeEpoch;
    controlState;
    onControlEvent;
    onFrameMapping;
    onLifecycle;
    socket = null;
    renewal = null;
    leaseWatchdog = null;
    stopped = false;
    reconnect;
    constructor(runtimeEpoch, controlState, onControlEvent, onFrameMapping = () => { }, onLifecycle = () => { }) {
        this.runtimeEpoch = runtimeEpoch;
        this.controlState = controlState;
        this.onControlEvent = onControlEvent;
        this.onFrameMapping = onFrameMapping;
        this.onLifecycle = onLifecycle;
        this.reconnect = new ReconnectLoop((connectionGeneration) => this.connect(connectionGeneration), {
            initialDelayMs: INITIAL_RECONNECT_DELAY_MS,
            maximumDelayMs: MAXIMUM_RECONNECT_DELAY_MS,
            stableAfterMs: STABLE_CONNECTION_MS,
            onRetryScheduled: (retryDelayMs) => {
                this.transition({
                    type: "retry_scheduled",
                    connectionGeneration: this.reconnect.connectionGeneration,
                    retryDelayMs
                });
            }
        });
    }
    start() {
        this.stopped = false;
        this.reconnect.start();
    }
    close() {
        this.stopped = true;
        this.reconnect.close();
        this.clearAuthorityTimers();
        this.socket?.close();
        this.socket = null;
    }
    retryNow() {
        const socket = this.socket;
        this.socket = null;
        socket?.close();
        this.clearAuthorityTimers();
        this.reconnect.retryNow();
    }
    toggleAuthority() {
        const state = this.controlState();
        if (state.connection !== "connected" || state.pendingIntent) {
            return;
        }
        this.transition({ type: "intent_queued" });
        const current = this.controlState();
        if (current.ownership === "you" && current.credentials !== null) {
            this.sendCredentials("release", current.credentials);
        }
        else {
            this.send({ schemaVersion: 1, type: "take" });
        }
    }
    setSettings(settings) {
        const state = this.controlState();
        if (!state.mayMutate || state.pendingIntent || state.credentials === null) {
            return;
        }
        this.transition({ type: "intent_queued", settings });
        this.send({
            schemaVersion: 1,
            type: "set_settings",
            generation: state.credentials.generation,
            secret: state.credentials.secret,
            settings
        });
    }
    markVisible(settingsGeneration) {
        this.transition({ type: "visible", settingsGeneration });
    }
    connect(connectionGeneration) {
        if (this.stopped) {
            return;
        }
        this.transition({ type: "connecting", connectionGeneration });
        const endpoint = new URL("/api/v1/control", window.location.href);
        endpoint.protocol = endpoint.protocol === "https:" ? "wss:" : "ws:";
        const socket = new WebSocket(endpoint);
        this.socket = socket;
        socket.addEventListener("open", () => {
            if (this.socket !== socket)
                return;
            this.reconnect.connected(connectionGeneration);
            this.transition({ type: "connected", connectionGeneration });
            const state = this.controlState();
            if (state.credentials !== null) {
                this.sendCredentials("resume", state.credentials);
            }
        });
        socket.addEventListener("message", (event) => {
            if (this.socket !== socket) {
                return;
            }
            if (typeof event.data !== "string") {
                return;
            }
            try {
                const value = JSON.parse(event.data);
                if (isRecord(value) && value.type === "frame_mapping") {
                    this.onFrameMapping(parseFrameMapping(value));
                }
                else if (isRecord(value) && value.type === "lifecycle") {
                    this.onLifecycle(value);
                }
                else {
                    this.acceptServerMessage(parseControlMessage(value));
                }
            }
            catch (error) {
                console.error("ObsCam control message rejected", error);
                socket.close();
            }
        });
        socket.addEventListener("close", () => {
            this.disconnectAndReconnect(socket, connectionGeneration, false);
        });
    }
    acceptServerMessage(message) {
        if (message.type === "granted") {
            this.transition({
                type: "granted",
                credentials: {
                    runtimeEpoch: this.runtimeEpoch(),
                    generation: message.generation,
                    secret: message.secret
                }
            });
            this.startRenewal();
            this.startLeaseWatchdog(message.leaseDurationMs);
            return;
        }
        this.transition(message);
        if (message.type === "resumed" || message.type === "renewed") {
            this.startRenewal();
            this.startLeaseWatchdog(message.leaseDurationMs);
        }
        else if (message.type === "released" ||
            (message.type === "rejected" && ["not_holder", "expired"].includes(message.reason))) {
            this.clearAuthorityTimers();
        }
        else if (message.type === "authority" && this.controlState().ownership !== "you") {
            this.clearAuthorityTimers();
        }
    }
    transition(event) {
        const transition = this.onControlEvent(event);
        persistCredentials(transition.storage, transition.state.credentials);
    }
    startRenewal() {
        if (this.renewal !== null) {
            return;
        }
        this.renewal = window.setInterval(() => {
            const state = this.controlState();
            if (state.mayMutate && state.credentials !== null) {
                this.sendCredentials("renew", state.credentials);
            }
        }, RENEWAL_INTERVAL_MS);
    }
    sendCredentials(type, value) {
        this.send({
            schemaVersion: 1,
            type,
            generation: value.generation,
            secret: value.secret
        });
    }
    send(value) {
        if (this.socket?.readyState === WebSocket.OPEN) {
            this.socket.send(JSON.stringify(value));
        }
    }
    clearRenewal() {
        if (this.renewal !== null) {
            window.clearInterval(this.renewal);
            this.renewal = null;
        }
    }
    startLeaseWatchdog(leaseDurationMs) {
        this.clearLeaseWatchdog();
        const failClosedAfterMs = Math.max(0, leaseDurationMs - RENEWAL_INTERVAL_MS);
        this.leaseWatchdog = window.setTimeout(() => this.failSilentControlConnection(), failClosedAfterMs);
    }
    failSilentControlConnection() {
        const socket = this.socket;
        if (socket !== null) {
            this.disconnectAndReconnect(socket, this.reconnect.connectionGeneration, true);
        }
    }
    disconnectAndReconnect(socket, connectionGeneration, closeSocket) {
        if (this.socket !== socket) {
            return;
        }
        this.socket = null;
        if (closeSocket) {
            socket.close();
        }
        this.clearAuthorityTimers();
        this.transition({ type: "disconnected", connectionGeneration });
        if (!this.stopped)
            this.reconnect.failed(connectionGeneration);
    }
    clearLeaseWatchdog() {
        if (this.leaseWatchdog !== null) {
            window.clearTimeout(this.leaseWatchdog);
            this.leaseWatchdog = null;
        }
    }
    clearAuthorityTimers() {
        this.clearRenewal();
        this.clearLeaseWatchdog();
    }
}
function withDerivedMutationPermission(state) {
    return {
        ...state,
        mayMutate: state.connection === "connected" && state.ownership === "you"
    };
}
function sameCameraSettings(left, right) {
    return left.exposureMs === right.exposureMs &&
        left.gain === right.gain &&
        left.treatment === right.treatment;
}
function rejectionDescription(reason) {
    return reason?.replaceAll("_", " ") ?? "control lost";
}
/** Reads and validates this tab's resumable same-runtime control credential. */
export function readStoredCredentials() {
    try {
        const value = JSON.parse(sessionStorage.getItem(STORAGE_KEY) ?? "null");
        return validCredentials(value) ? value : null;
    }
    catch {
        return null;
    }
}
function persistCredentials(action, credentials) {
    try {
        if (action === "save" && credentials !== null) {
            sessionStorage.setItem(STORAGE_KEY, JSON.stringify(credentials));
        }
        else if (action === "remove") {
            sessionStorage.removeItem(STORAGE_KEY);
        }
    }
    catch (error) {
        console.error("ObsCam tab credential storage unavailable", error);
    }
}
function validCredentials(value) {
    return (isRecord(value) &&
        typeof value.runtimeEpoch === "string" &&
        validGeneration(value.generation) &&
        validSecret(value.secret));
}
function validGeneration(value, zeroAllowed = false) {
    return (typeof value === "number" &&
        Number.isSafeInteger(value) &&
        value >= (zeroAllowed ? 0 : 1));
}
function validSecret(value) {
    return typeof value === "string" && /^[0-9a-f]{64}$/u.test(value);
}
function parseSettingsState(value) {
    if (!isRecord(value)) {
        return null;
    }
    const applied = parseVersionedSettings(value.applied, true);
    const pending = value.pending === null ? null : parseVersionedSettings(value.pending, false);
    return applied !== null && (value.pending === null || pending !== null)
        ? { applied, pending, visible: null, presentedGeneration: null }
        : null;
}
function parseVersionedSettings(value, zeroAllowed) {
    if (!isRecord(value) || !validGeneration(value.generation, zeroAllowed)) {
        return null;
    }
    const settings = parseCameraSettings(value.settings);
    return settings === null ? null : { generation: value.generation, settings };
}
function parseCameraSettings(value) {
    if (!isRecord(value) ||
        typeof value.exposureMs !== "number" ||
        !EXPOSURE_CHOICES_MS.has(value.exposureMs) ||
        typeof value.gain !== "number" ||
        !Number.isInteger(value.gain) ||
        value.gain < 0 ||
        value.gain > 600 ||
        value.gain % 50 !== 0 ||
        (value.treatment !== "monochrome" && value.treatment !== "colour")) {
        return null;
    }
    return { exposureMs: value.exposureMs, gain: value.gain, treatment: value.treatment };
}
function isRejectionReason(value) {
    return [
        "not_holder",
        "expired",
        "malformed",
        "unsupported_schema",
        "invalid_settings",
        "camera_unavailable"
    ].includes(String(value));
}
function isRecord(value) {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}
