import assert from "node:assert/strict";
import test from "node:test";

import {
  initialPresentationState,
  parseFrameMapping,
  reducePresentation
} from "../dist/presentation.js";

const runtimeEpoch = "8d4cc9fd-b91f-4d0c-a2e3-0f3838608262";
const mapping = {
  schemaVersion: 1,
  type: "frame_mapping",
  runtimeEpoch,
  streamEpoch: 2,
  rtpTimestamp: 4_294_965_296,
  sourceGeneration: 81,
  settingsGeneration: 7,
  treatment: "monochrome",
  width: 1920,
  height: 1080,
  exposureCompletedAtUnixUs: 1_700_000_000_000_000,
  submittedAtUnixUs: 1_700_000_000_010_000,
  repeat: false
};

test("a callback matches only exact retained RTP metadata", () => {
  let state = initialPresentationState(runtimeEpoch, 2);
  state = reducePresentation(state, { type: "mapping", mapping: parseFrameMapping(mapping) }).state;

  assert.equal(
    reducePresentation(state, { type: "presented", rtpTimestamp: mapping.rtpTimestamp, nowUnixUs: mapping.submittedAtUnixUs + 50_000 }).presented?.sourceGeneration,
    81
  );
  assert.equal(
    reducePresentation(state, { type: "presented", nowUnixUs: mapping.submittedAtUnixUs + 50_000 }).presented,
    null
  );
  assert.equal(
    reducePresentation(state, { type: "presented", rtpTimestamp: 123, nowUnixUs: mapping.submittedAtUnixUs + 50_000 }).presented,
    null
  );
});

test("epoch changes, conflicts, and stale mappings fail closed", () => {
  let state = initialPresentationState(runtimeEpoch, 2);
  state = reducePresentation(state, { type: "mapping", mapping: parseFrameMapping(mapping) }).state;
  state = reducePresentation(state, {
    type: "mapping",
    mapping: parseFrameMapping({ ...mapping, sourceGeneration: 82 })
  }).state;
  assert.equal(
    reducePresentation(state, { type: "presented", rtpTimestamp: mapping.rtpTimestamp, nowUnixUs: mapping.submittedAtUnixUs + 1 }).presented,
    null
  );

  state = reducePresentation(state, {
    type: "mapping",
    mapping: parseFrameMapping({ ...mapping, streamEpoch: 3, rtpTimestamp: 22 })
  }).state;
  assert.equal(state.streamEpoch, 3);
  assert.equal(
    reducePresentation(state, { type: "presented", rtpTimestamp: mapping.rtpTimestamp, nowUnixUs: mapping.submittedAtUnixUs + 1 }).presented,
    null
  );
  assert.equal(
    reducePresentation(state, { type: "presented", rtpTimestamp: 22, nowUnixUs: mapping.submittedAtUnixUs + 5_000_001 }).presented,
    null
  );
});

test("mapping validation rejects epoch mismatch and invalid dimensions", () => {
  assert.throws(() => parseFrameMapping({ ...mapping, runtimeEpoch: "wrong" }), /invalid frame mapping/);
  assert.throws(() => parseFrameMapping({ ...mapping, streamEpoch: 0 }), /invalid frame mapping/);
  assert.throws(() => parseFrameMapping({ ...mapping, width: 1280 }), /invalid frame mapping/);
});

test("reconnect clears retained mappings and accepts only the new component epoch", () => {
  let state = initialPresentationState(runtimeEpoch, 2);
  state = reducePresentation(state, { type: "mapping", mapping: parseFrameMapping(mapping) }).state;
  state = reducePresentation(state, { type: "reconnected", streamEpoch: 3 }).state;

  assert.equal(state.streamEpoch, 3);
  assert.equal(state.mappings.length, 0);
  assert.equal(
    reducePresentation(state, { type: "presented", rtpTimestamp: mapping.rtpTimestamp, nowUnixUs: mapping.submittedAtUnixUs + 1 }).presented,
    null
  );
});

test("browser mappings and conflict evidence remain bounded at capacity", () => {
  let state = initialPresentationState(runtimeEpoch, 2);
  for (let index = 0; index < 130; index += 1) {
    state = reducePresentation(state, {
      type: "mapping",
      mapping: parseFrameMapping({
        ...mapping,
        rtpTimestamp: index,
        sourceGeneration: index + 1
      })
    }).state;
  }
  assert.equal(state.mappings.length, 128);
  assert.equal(
    reducePresentation(state, { type: "presented", rtpTimestamp: 0, nowUnixUs: mapping.submittedAtUnixUs + 1 }).presented,
    null
  );

  for (let index = 2; index < 130; index += 1) {
    state = reducePresentation(state, {
      type: "mapping",
      mapping: parseFrameMapping({
        ...mapping,
        rtpTimestamp: index,
        sourceGeneration: index + 10_000
      })
    }).state;
  }
  assert.equal(state.poisonedTimestamps.length, 128);
});
