/** Fixed reasons that justify an unavailable runtime component claim. */
export type UnavailableReason = "no_camera_source" | "no_frame" | "not_observed";

/** One authoritative Rust component fact at a browser contract boundary. */
export type ComponentStatus =
  | { state: "ready" }
  | { state: "unavailable"; reason: UnavailableReason };

/** Independently reported capture, encoder, and currently-unobserved relay facts. */
export interface RuntimeComponents {
  capture: ComponentStatus;
  encoder: ComponentStatus;
  relay: ComponentStatus;
}

/** Runtime-local counters that distinguish child replacement from dropped work. */
export interface MediaRecoveryCounters {
  encoderReplacements: number;
  pipelineSkips: number;
  cameraRestarts: number;
  invalidDimensions: number;
  invalidBufferLengths: number;
  invalidGenerationMetadata: number;
  invalidProcessingOutput: number;
}

/** Version-one bootstrap facts supplied by the Rust service. */
export interface RuntimeContract {
  schemaVersion: 1;
  runtimeEpoch: string;
  minimumSourceGeneration: number;
  media: {
    whepPort: number;
    whepPath: string;
  };
  components: RuntimeComponents;
  mediaRecovery: MediaRecoveryCounters;
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
    typeof value.minimumSourceGeneration !== "number" ||
    !nonNegativeSafeInteger(value.minimumSourceGeneration) ||
    value.minimumSourceGeneration < 1
  ) {
    throw new Error("invalid minimum source generation");
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
  const components = parseRuntimeComponents(value.components);
  const mediaRecovery = parseMediaRecoveryCounters(value.mediaRecovery);
  if (value.latestFrame !== null) {
    throw new Error("untrusted frame contract");
  }

  return {
    schemaVersion: 1,
    runtimeEpoch: value.runtimeEpoch,
    minimumSourceGeneration: value.minimumSourceGeneration as number,
    media: {
      whepPort: value.media.whepPort,
      whepPath: value.media.whepPath
    },
    components,
    mediaRecovery,
    latestFrame: null
  };
}

function parseMediaRecoveryCounters(value: unknown): MediaRecoveryCounters {
  if (
    !isRecord(value) ||
    !nonNegativeSafeInteger(value.encoderReplacements) ||
    !nonNegativeSafeInteger(value.pipelineSkips) ||
    !nonNegativeSafeInteger(value.cameraRestarts) ||
    !nonNegativeSafeInteger(value.invalidDimensions) ||
    !nonNegativeSafeInteger(value.invalidBufferLengths) ||
    !nonNegativeSafeInteger(value.invalidGenerationMetadata) ||
    !nonNegativeSafeInteger(value.invalidProcessingOutput)
  ) {
    throw new Error("invalid media recovery counters");
  }
  return {
    encoderReplacements: value.encoderReplacements as number,
    pipelineSkips: value.pipelineSkips as number,
    cameraRestarts: value.cameraRestarts as number,
    invalidDimensions: value.invalidDimensions as number,
    invalidBufferLengths: value.invalidBufferLengths as number,
    invalidGenerationMetadata: value.invalidGenerationMetadata as number,
    invalidProcessingOutput: value.invalidProcessingOutput as number
  };
}

function nonNegativeSafeInteger(value: unknown): boolean {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

/** Validates and normalizes independently reported component facts. */
export function parseRuntimeComponents(value: unknown): RuntimeComponents {
  if (
    !isRecord(value) ||
    !isComponentStatus(value.capture, "no_camera_source") ||
    !isComponentStatus(value.encoder, "no_frame") ||
    !isComponentStatus(value.relay, "not_observed")
  ) {
    throw new Error("invalid component state");
  }
  return {
    capture: parseComponentStatus(value.capture, "no_camera_source"),
    encoder: parseComponentStatus(value.encoder, "no_frame"),
    relay: parseComponentStatus(value.relay, "not_observed")
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
