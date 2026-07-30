type UnavailableReason = "no_camera_source" | "no_frame" | "not_observed";

type ComponentStatus =
  | { state: "ready" }
  | { state: "unavailable"; reason: UnavailableReason };

/** Version-one bootstrap facts supplied by the Rust service. */
export interface RuntimeContract {
  schemaVersion: 1;
  runtimeEpoch: string;
  media: {
    whepPort: number;
    whepPath: string;
  };
  components: {
    capture: ComponentStatus;
    encoder: ComponentStatus;
    relay: ComponentStatus;
  };
  latestFrame: null;
}

/** Browser-visible state; frame-derived facts are absent until proven. */
export interface ViewerState {
  status: "Unavailable";
  frameAgeMs?: number;
  captureCadenceHz?: number;
  sourceGeneration?: number;
  visibleLatencyMs?: number;
}

/** Derives a direct WHEP endpoint without accepting a media-supplied authority. */
export function deriveWhepUrl(
  media: RuntimeContract["media"],
  pageUrl: string
): string {
  const endpoint = new URL(pageUrl);
  endpoint.port = String(media.whepPort);
  endpoint.pathname = media.whepPath;
  endpoint.search = "";
  endpoint.hash = "";
  return endpoint.toString();
}

/** Validates the untrusted runtime payload and rejects unsupported frame claims. */
export function parseRuntimeContract(value: unknown): RuntimeContract {
  if (!isRecord(value) || value.schemaVersion !== 1) {
    throw new Error("unsupported runtime contract");
  }
  if (
    typeof value.runtimeEpoch !== "string" ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu.test(
      value.runtimeEpoch
    )
  ) {
    throw new Error("invalid runtime epoch");
  }
  if (
    !isRecord(value.media) ||
    !Number.isInteger(value.media.whepPort) ||
    typeof value.media.whepPort !== "number" ||
    value.media.whepPort < 1 ||
    value.media.whepPort > 65_535 ||
    typeof value.media.whepPath !== "string" ||
    !isHostlessWhepPath(value.media.whepPath)
  ) {
    throw new Error("invalid media descriptor");
  }
  if (
    !isRecord(value.components) ||
    !isComponentStatus(value.components.capture, "no_camera_source") ||
    !isComponentStatus(value.components.encoder, "no_frame") ||
    !isComponentStatus(value.components.relay, "not_observed")
  ) {
    throw new Error("invalid component state");
  }
  if (value.latestFrame !== null) {
    throw new Error("untrusted frame contract");
  }

  return {
    schemaVersion: 1,
    runtimeEpoch: value.runtimeEpoch,
    media: {
      whepPort: value.media.whepPort,
      whepPath: value.media.whepPath
    },
    components: {
      capture: parseComponentStatus(value.components.capture, "no_camera_source"),
      encoder: parseComponentStatus(value.components.encoder, "no_frame"),
      relay: parseComponentStatus(value.components.relay, "not_observed")
    },
    latestFrame: null
  };
}

/** Derives only claims justified by the validated bootstrap contract. */
export function deriveViewerState(_runtime: RuntimeContract): ViewerState {
  return { status: "Unavailable" };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isHostlessWhepPath(value: string): boolean {
  return (
    value.startsWith("/") &&
    !value.startsWith("//") &&
    !/[\s?#]/u.test(value) &&
    new TextEncoder().encode(value).byteLength <= 256
  );
}

function isComponentStatus(value: unknown, reason: UnavailableReason): boolean {
  return (
    isRecord(value) &&
    (value.state === "ready" || (value.state === "unavailable" && value.reason === reason))
  );
}

function parseComponentStatus(
  value: unknown,
  reason: UnavailableReason
): ComponentStatus {
  if (!isRecord(value) || value.state === "ready") {
    return { state: "ready" };
  }
  return { state: "unavailable", reason };
}
