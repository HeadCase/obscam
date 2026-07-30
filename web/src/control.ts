const STORAGE_KEY = "obscam.control.v1";
const LEASE_DURATION_MS = 5_000;
const RENEWAL_INTERVAL_MS = 2_000;
const RECONNECT_DELAY_MS = 500;

export interface StoredCredentials {
  runtimeEpoch: string;
  generation: number;
  secret: string;
}

export interface ControlState {
  connection: "connected" | "disconnected";
  ownership: "no_one" | "another_viewer" | "you";
  generation: number;
  credentials: StoredCredentials | null;
  credentialsValidated: boolean;
  pendingIntent: boolean;
  mayMutate: boolean;
}

export type ControlEvent =
  | { type: "connected" }
  | { type: "disconnected" }
  | { type: "authority"; state: "unheld" | "held"; generation: number }
  | { type: "granted"; credentials: StoredCredentials }
  | { type: "resumed"; generation: number }
  | { type: "renewed"; generation: number }
  | { type: "released"; generation: number }
  | { type: "rejected" }
  | { type: "intent_queued" };

export interface ControlTransition {
  state: ControlState;
  storage: "none" | "save" | "remove";
}

type ControlMessage =
  | { type: "authority"; state: "unheld" | "held"; generation: number }
  | {
      type: "granted";
      generation: number;
      secret: string;
      leaseDurationMs: typeof LEASE_DURATION_MS;
    }
  | {
      type: "resumed" | "renewed";
      generation: number;
      leaseDurationMs: typeof LEASE_DURATION_MS;
    }
  | { type: "released"; generation: number }
  | { type: "rejected" };

export function initialControlState(
  stored: StoredCredentials | null,
  runtimeEpoch: string = stored?.runtimeEpoch ?? ""
): ControlState {
  const credentials =
    stored !== null && stored.runtimeEpoch === runtimeEpoch && validCredentials(stored)
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

export function reduceControl(state: ControlState, event: ControlEvent): ControlTransition {
  let storage: ControlTransition["storage"] = "none";
  let next: Omit<ControlState, "mayMutate"> = state;

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

export function parseControlMessage(value: unknown): ControlMessage {
  if (!isRecord(value) || value.schemaVersion !== 1 || typeof value.type !== "string") {
    throw new Error("unsupported control message");
  }
  if (value.type === "authority") {
    if ((value.state === "held" || value.state === "unheld") && validGeneration(value.generation, true)) {
      return { type: "authority", state: value.state, generation: value.generation };
    }
  } else if (value.type === "granted") {
    if (
      validGeneration(value.generation) &&
      validSecret(value.secret) &&
      value.leaseDurationMs === LEASE_DURATION_MS
    ) {
      return {
        type: "granted",
        generation: value.generation,
        secret: value.secret,
        leaseDurationMs: value.leaseDurationMs
      };
    }
  } else if (value.type === "renewed" || value.type === "resumed") {
    if (validGeneration(value.generation) && value.leaseDurationMs === LEASE_DURATION_MS) {
      return {
        type: value.type,
        generation: value.generation,
        leaseDurationMs: value.leaseDurationMs
      };
    }
  } else if (value.type === "released") {
    if (validGeneration(value.generation)) {
      return { type: "released", generation: value.generation };
    }
  } else if (value.type === "rejected") {
    if (["not_holder", "expired", "malformed", "unsupported_schema"].includes(String(value.reason))) {
      return { type: "rejected" };
    }
  }
  throw new Error("invalid control message");
}

/** Owns the browser effects around the pure control reducer. */
export class ControlClient {
  private state: ControlState;
  private socket: WebSocket | null = null;
  private renewal: number | null = null;
  private leaseWatchdog: number | null = null;
  private reconnect: number | null = null;
  private stopped = false;

  constructor(
    private readonly runtimeEpoch: string,
    private readonly onState: (state: ControlState) => void
  ) {
    this.state = initialControlState(readStoredCredentials(), runtimeEpoch);
  }

  start(): void {
    this.stopped = false;
    this.connect();
    this.onState(this.state);
  }

  close(): void {
    this.stopped = true;
    this.clearTimers();
    this.socket?.close();
    this.socket = null;
  }

  toggleAuthority(): void {
    if (this.state.connection !== "connected" || this.state.pendingIntent) {
      return;
    }
    this.transition({ type: "intent_queued" });
    if (this.state.ownership === "you" && this.state.credentials !== null) {
      this.sendCredentials("release", this.state.credentials);
    } else {
      this.send({ schemaVersion: 1, type: "take" });
    }
  }

  private connect(): void {
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
    socket.addEventListener("message", (event: MessageEvent<unknown>) => {
      if (this.socket !== socket) {
        return;
      }
      if (typeof event.data !== "string") {
        return;
      }
      try {
        this.acceptServerMessage(parseControlMessage(JSON.parse(event.data)));
      } catch (error: unknown) {
        console.error("ObsCam control message rejected", error);
        socket.close();
      }
    });
    socket.addEventListener("close", () => {
      this.disconnectAndReconnect(socket, false);
    });
  }

  private acceptServerMessage(message: ControlMessage): void {
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
    } else if (message.type === "rejected" || message.type === "released") {
      this.clearAuthorityTimers();
    } else if (message.type === "authority" && this.state.ownership !== "you") {
      this.clearAuthorityTimers();
    }
  }

  private transition(event: ControlEvent): void {
    const transition = reduceControl(this.state, event);
    this.state = transition.state;
    persistCredentials(transition.storage, this.state.credentials);
    this.onState(this.state);
  }

  private startRenewal(): void {
    if (this.renewal !== null) {
      return;
    }
    this.renewal = window.setInterval(() => {
      if (this.state.mayMutate && this.state.credentials !== null) {
        this.sendCredentials("renew", this.state.credentials);
      }
    }, RENEWAL_INTERVAL_MS);
  }

  private sendCredentials(type: "renew" | "resume" | "release", value: StoredCredentials): void {
    this.send({
      schemaVersion: 1,
      type,
      generation: value.generation,
      secret: value.secret
    });
  }

  private send(value: object): void {
    if (this.socket?.readyState === WebSocket.OPEN) {
      this.socket.send(JSON.stringify(value));
    }
  }

  private clearRenewal(): void {
    if (this.renewal !== null) {
      window.clearInterval(this.renewal);
      this.renewal = null;
    }
  }

  private startLeaseWatchdog(leaseDurationMs: number): void {
    this.clearLeaseWatchdog();
    const failClosedAfterMs = Math.max(0, leaseDurationMs - RENEWAL_INTERVAL_MS);
    this.leaseWatchdog = window.setTimeout(() => this.failSilentControlConnection(), failClosedAfterMs);
  }

  private failSilentControlConnection(): void {
    const socket = this.socket;
    if (socket !== null) {
      this.disconnectAndReconnect(socket, true);
    }
  }

  private disconnectAndReconnect(socket: WebSocket, closeSocket: boolean): void {
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

  private clearLeaseWatchdog(): void {
    if (this.leaseWatchdog !== null) {
      window.clearTimeout(this.leaseWatchdog);
      this.leaseWatchdog = null;
    }
  }

  private clearAuthorityTimers(): void {
    this.clearRenewal();
    this.clearLeaseWatchdog();
  }

  private clearTimers(): void {
    this.clearAuthorityTimers();
    if (this.reconnect !== null) {
      window.clearTimeout(this.reconnect);
      this.reconnect = null;
    }
  }
}

function withDerivedMutationPermission(
  state: Omit<ControlState, "mayMutate">
): ControlState {
  return {
    ...state,
    mayMutate: state.connection === "connected" && state.ownership === "you"
  };
}

function readStoredCredentials(): StoredCredentials | null {
  try {
    const value: unknown = JSON.parse(sessionStorage.getItem(STORAGE_KEY) ?? "null");
    return validCredentials(value) ? value : null;
  } catch {
    return null;
  }
}

function persistCredentials(
  action: ControlTransition["storage"],
  credentials: StoredCredentials | null
): void {
  try {
    if (action === "save" && credentials !== null) {
      sessionStorage.setItem(STORAGE_KEY, JSON.stringify(credentials));
    } else if (action === "remove") {
      sessionStorage.removeItem(STORAGE_KEY);
    }
  } catch (error: unknown) {
    console.error("ObsCam tab credential storage unavailable", error);
  }
}

function validCredentials(value: unknown): value is StoredCredentials {
  return (
    isRecord(value) &&
    typeof value.runtimeEpoch === "string" &&
    validGeneration(value.generation) &&
    validSecret(value.secret)
  );
}

function validGeneration(value: unknown, zeroAllowed = false): value is number {
  return (
    typeof value === "number" &&
    Number.isSafeInteger(value) &&
    value >= (zeroAllowed ? 0 : 1)
  );
}

function validSecret(value: unknown): value is string {
  return typeof value === "string" && /^[0-9a-f]{64}$/u.test(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
