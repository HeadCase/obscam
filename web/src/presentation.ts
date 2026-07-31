const MAX_MAPPINGS = 128;
// The qualified Pi hardware pipeline can present an exact RTP mapping just over
// one second after FFmpeg submission. Stream/media-generation fences and the
// bounded mapping window still reject callbacks from prior sessions.
const MAX_MAPPING_AGE_US = 2_000_000;

export interface FrameMapping {
  runtimeEpoch: string;
  streamEpoch: number;
  rtpTimestamp: number;
  sourceGeneration: number;
  settingsGeneration: number;
  treatment: "monochrome" | "colour";
  width: 1920;
  height: 1080;
  exposureCompletedAtUnixUs: number;
  submittedAtUnixUs: number;
  repeat: boolean;
}

export interface PresentationState {
  runtimeEpoch: string;
  streamEpoch: number;
  mappings: readonly FrameMapping[];
  poisonedTimestamps: readonly number[];
}

export type PresentationEvent =
  | { type: "mapping"; mapping: FrameMapping }
  | { type: "presented"; rtpTimestamp?: number; nowUnixUs: number }
  | { type: "reconnected"; streamEpoch: number };

export interface PresentationTransition {
  state: PresentationState;
  presented: FrameMapping | null;
}

export function initialPresentationState(
  runtimeEpoch: string,
  streamEpoch = 0
): PresentationState {
  return { runtimeEpoch, streamEpoch, mappings: [], poisonedTimestamps: [] };
}

export function reducePresentation(
  state: PresentationState,
  event: PresentationEvent
): PresentationTransition {
  if (event.type === "reconnected") {
    return {
      state: initialPresentationState(state.runtimeEpoch, event.streamEpoch),
      presented: null
    };
  }
  if (event.type === "mapping") {
    const mapping = event.mapping;
    if (mapping.runtimeEpoch !== state.runtimeEpoch || mapping.streamEpoch < state.streamEpoch) {
      return { state, presented: null };
    }
    const current =
      mapping.streamEpoch === state.streamEpoch
        ? state
        : initialPresentationState(state.runtimeEpoch, mapping.streamEpoch);
    const conflicting = current.mappings.find(
      (candidate) => candidate.rtpTimestamp === mapping.rtpTimestamp
    );
    if (conflicting !== undefined) {
      if (sameMapping(conflicting, mapping)) {
        return { state: current, presented: null };
      }
      const poisoned = [...current.poisonedTimestamps, mapping.rtpTimestamp].slice(-MAX_MAPPINGS);
      return {
        state: {
          ...current,
          mappings: current.mappings.filter(
            (candidate) => candidate.rtpTimestamp !== mapping.rtpTimestamp
          ),
          poisonedTimestamps: poisoned
        },
        presented: null
      };
    }
    if (current.poisonedTimestamps.includes(mapping.rtpTimestamp)) {
      return { state: current, presented: null };
    }
    return {
      state: {
        ...current,
        mappings: [...current.mappings, mapping].slice(-MAX_MAPPINGS)
      },
      presented: null
    };
  }

  if (event.rtpTimestamp === undefined || state.poisonedTimestamps.includes(event.rtpTimestamp)) {
    return { state, presented: null };
  }
  const mapping = state.mappings.find(
    (candidate) => candidate.rtpTimestamp === event.rtpTimestamp
  );
  const exact =
    mapping !== undefined &&
    event.nowUnixUs >= mapping.submittedAtUnixUs &&
    event.nowUnixUs - mapping.submittedAtUnixUs <= MAX_MAPPING_AGE_US;
  return { state, presented: exact ? mapping : null };
}

export function parseFrameMapping(value: unknown): FrameMapping {
  if (
    !isRecord(value) ||
    value.schemaVersion !== 1 ||
    value.type !== "frame_mapping" ||
    !isUuid(value.runtimeEpoch) ||
    !validGeneration(value.streamEpoch) ||
    !validRtpTimestamp(value.rtpTimestamp) ||
    !validGeneration(value.sourceGeneration) ||
    !validGeneration(value.settingsGeneration, true) ||
    (value.treatment !== "monochrome" && value.treatment !== "colour") ||
    value.width !== 1920 ||
    value.height !== 1080 ||
    !validTimestamp(value.exposureCompletedAtUnixUs) ||
    !validTimestamp(value.submittedAtUnixUs) ||
    value.submittedAtUnixUs < value.exposureCompletedAtUnixUs ||
    typeof value.repeat !== "boolean"
  ) {
    throw new Error("invalid frame mapping");
  }
  return {
    runtimeEpoch: value.runtimeEpoch,
    streamEpoch: value.streamEpoch,
    rtpTimestamp: value.rtpTimestamp,
    sourceGeneration: value.sourceGeneration,
    settingsGeneration: value.settingsGeneration,
    treatment: value.treatment,
    width: value.width,
    height: value.height,
    exposureCompletedAtUnixUs: value.exposureCompletedAtUnixUs,
    submittedAtUnixUs: value.submittedAtUnixUs,
    repeat: value.repeat
  };
}

function sameMapping(left: FrameMapping, right: FrameMapping): boolean {
  return Object.keys(left).every(
    (key) => left[key as keyof FrameMapping] === right[key as keyof FrameMapping]
  );
}

function validGeneration(value: unknown, zeroAllowed = false): value is number {
  return (
    typeof value === "number" &&
    Number.isSafeInteger(value) &&
    value >= (zeroAllowed ? 0 : 1)
  );
}

function validRtpTimestamp(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= 0xffff_ffff;
}

function validTimestamp(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isUuid(value: unknown): value is string {
  return (
    typeof value === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu.test(value)
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
