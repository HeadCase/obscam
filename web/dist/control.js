const STORAGE_KEY = "obscam.control.v1";
const LEASE_DURATION_MS = 5_000;
const RENEWAL_INTERVAL_MS = 2_000;
const RECONNECT_DELAY_MS = 500;
export function initialControlState(stored, runtimeEpoch = stored?.runtimeEpoch ?? "") {
    const credentials = stored !== null && stored.runtimeEpoch === runtimeEpoch && validCredentials(stored)
        ? stored
        : null;
    return withDerivedMutationPermission({
        connection: "disconnected",
        ownership: "no_one",
        generation: credentials?.generation ?? 0,
        credentials,
        credentialsValidated: false,
        pendingIntent: false
    });
}
export function reduceControl(state, event) {
    let storage = "none";
    let next = state;
    switch (event.type) {
        case "connected":
            next = { ...state, connection: "connected" };
            break;
        case "disconnected":
            next = {
                ...state,
                connection: "disconnected",
                ownership: "no_one",
                credentialsValidated: false,
                pendingIntent: false
            };
            break;
        case "granted":
            next = {
                ...state,
                ownership: "you",
                generation: event.credentials.generation,
                credentials: event.credentials,
                credentialsValidated: true,
                pendingIntent: false
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
                pendingIntent: false
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
                    pendingIntent: false
                };
                storage = state.credentials === null ? "none" : "remove";
            }
            break;
        case "rejected":
            next = {
                ...state,
                ownership: state.ownership === "another_viewer" ? "another_viewer" : "no_one",
                credentials: null,
                credentialsValidated: false,
                pendingIntent: false
            };
            storage = state.credentials === null ? "none" : "remove";
            break;
        case "intent_queued":
            next = { ...state, pendingIntent: state.mayMutate };
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
    else if (value.type === "rejected") {
        if (["not_holder", "expired", "malformed", "unsupported_schema"].includes(String(value.reason))) {
            return { type: "rejected" };
        }
    }
    throw new Error("invalid control message");
}
/** Owns the browser effects around the pure control reducer. */
export class ControlClient {
    runtimeEpoch;
    onState;
    state;
    socket = null;
    renewal = null;
    leaseWatchdog = null;
    reconnect = null;
    stopped = false;
    constructor(runtimeEpoch, onState) {
        this.runtimeEpoch = runtimeEpoch;
        this.onState = onState;
        this.state = initialControlState(readStoredCredentials(), runtimeEpoch);
    }
    start() {
        this.stopped = false;
        this.connect();
        this.onState(this.state);
    }
    close() {
        this.stopped = true;
        this.clearTimers();
        this.socket?.close();
        this.socket = null;
    }
    toggleAuthority() {
        if (this.state.connection !== "connected" || this.state.pendingIntent) {
            return;
        }
        this.transition({ type: "intent_queued" });
        if (this.state.ownership === "you" && this.state.credentials !== null) {
            this.sendCredentials("release", this.state.credentials);
        }
        else {
            this.send({ schemaVersion: 1, type: "take" });
        }
    }
    connect() {
        if (this.stopped) {
            return;
        }
        this.reconnect = null;
        const endpoint = new URL("/api/v1/control", window.location.href);
        endpoint.protocol = endpoint.protocol === "https:" ? "wss:" : "ws:";
        const socket = new WebSocket(endpoint);
        this.socket = socket;
        socket.addEventListener("open", () => {
            this.transition({ type: "connected" });
            if (this.state.credentials !== null) {
                this.sendCredentials("resume", this.state.credentials);
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
                this.acceptServerMessage(parseControlMessage(JSON.parse(event.data)));
            }
            catch (error) {
                console.error("ObsCam control message rejected", error);
                socket.close();
            }
        });
        socket.addEventListener("close", () => {
            this.disconnectAndReconnect(socket, false);
        });
    }
    acceptServerMessage(message) {
        if (message.type === "granted") {
            this.transition({
                type: "granted",
                credentials: {
                    runtimeEpoch: this.runtimeEpoch,
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
        else if (message.type === "rejected" || message.type === "released") {
            this.clearAuthorityTimers();
        }
        else if (message.type === "authority" && this.state.ownership !== "you") {
            this.clearAuthorityTimers();
        }
    }
    transition(event) {
        const transition = reduceControl(this.state, event);
        this.state = transition.state;
        persistCredentials(transition.storage, this.state.credentials);
        this.onState(this.state);
    }
    startRenewal() {
        if (this.renewal !== null) {
            return;
        }
        this.renewal = window.setInterval(() => {
            if (this.state.mayMutate && this.state.credentials !== null) {
                this.sendCredentials("renew", this.state.credentials);
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
            this.disconnectAndReconnect(socket, true);
        }
    }
    disconnectAndReconnect(socket, closeSocket) {
        if (this.socket !== socket) {
            return;
        }
        this.socket = null;
        if (closeSocket) {
            socket.close();
        }
        this.clearAuthorityTimers();
        this.transition({ type: "disconnected" });
        if (!this.stopped) {
            this.reconnect = window.setTimeout(() => this.connect(), RECONNECT_DELAY_MS);
        }
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
    clearTimers() {
        this.clearAuthorityTimers();
        if (this.reconnect !== null) {
            window.clearTimeout(this.reconnect);
            this.reconnect = null;
        }
    }
}
function withDerivedMutationPermission(state) {
    return {
        ...state,
        mayMutate: state.connection === "connected" && state.ownership === "you"
    };
}
function readStoredCredentials() {
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
function isRecord(value) {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}
