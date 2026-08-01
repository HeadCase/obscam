import assert from "node:assert/strict";
import test from "node:test";

import {
  acceptsMediaPresentation,
  initialViewerState,
  parseLifecycleFacts,
  reduceViewer,
  viewerProjection
} from "../dist/viewer.js";
import { parseFrameMapping } from "../dist/presentation.js";

const runtimeEpoch = "8d4cc9fd-b91f-4d0c-a2e3-0f3838608262";
const nextRuntimeEpoch = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const startedAtUnixUs = 1_700_000_000_000_000;
const ready = {
  runtimeEpoch,
  components: {
    capture: { state: "ready" },
    encoder: { state: "ready" },
    relay: { state: "ready" }
  },
  recovery: null,
  capture: {
    settingsGeneration: 0,
    exposureMs: 500,
    startedAtUnixUs
  }
};
const mapping = parseFrameMapping({
  schemaVersion: 1,
  type: "frame_mapping",
  runtimeEpoch,
  streamEpoch: 2,
  rtpTimestamp: 42,
  sourceGeneration: 81,
  settingsGeneration: 0,
  treatment: "monochrome",
  width: 1920,
  height: 1080,
  exposureCompletedAtUnixUs: startedAtUnixUs + 500_000,
  submittedAtUnixUs: startedAtUnixUs + 510_000,
  repeat: false
});
const evidence = { schemaVersion: 1, limits: { clients: 16, samplesPerClient: 512 }, clients: [], combined: {} };

function dispatch(state, event) {
  const generationEvent = event.type === "presented" && event.mediaConnectionGeneration === undefined
    ? { ...event, mediaConnectionGeneration: state.mediaConnectionGeneration }
    : event;
  return reduceViewer(state, generationEvent).state;
}

function readyState() {
  return dispatch(initialViewerState(runtimeEpoch), { type: "lifecycle", facts: ready });
}

function presentedState() {
  let state = readyState();
  state = dispatch(state, { type: "media_connected" });
  state = dispatch(state, { type: "mapping", mapping });
  return dispatch(state, {
    type: "presented",
    rtpTimestamp: mapping.rtpTimestamp,
    nowUnixUs: mapping.submittedAtUnixUs + 20_000
  });
}

test("primary media states follow authoritative progress and exact presentation", () => {
  let state = initialViewerState(runtimeEpoch);
  assert.equal(viewerProjection(state).status, "Reconnecting");

  state = dispatch(state, { type: "lifecycle", facts: ready });
  assert.equal(viewerProjection(state).status, "Capturing");

  state = dispatch(state, { type: "media_connected" });
  state = dispatch(state, { type: "mapping", mapping });
  state = dispatch(state, {
    type: "presented",
    rtpTimestamp: mapping.rtpTimestamp,
    nowUnixUs: mapping.submittedAtUnixUs + 20_000
  });
  assert.equal(viewerProjection(state).status, "Live");
  assert.equal(viewerProjection(state).sourceGeneration, 81);
});

test("lifecycle facts fail closed at the browser boundary", () => {
  assert.deepEqual(
    parseLifecycleFacts({ schemaVersion: 1, type: "lifecycle", ...ready }),
    ready
  );
  assert.throws(
    () => parseLifecycleFacts({
      schemaVersion: 1,
      type: "lifecycle",
      ...ready,
      capture: { ...ready.capture, exposureMs: 31_000 }
    }),
    /invalid lifecycle facts/
  );
  assert.throws(
    () => parseLifecycleFacts({ schemaVersion: 1, type: "lifecycle", ...ready, runtimeEpoch: "old" }),
    /invalid lifecycle facts/
  );
});

test("a progressing long exposure retains the trustworthy frame as Capturing", () => {
  let state = presentedState();
  const nextStartedAtUnixUs = mapping.exposureCompletedAtUnixUs + 1;
  state = dispatch(state, {
    type: "lifecycle",
    facts: {
      ...ready,
      capture: {
        settingsGeneration: 0,
        exposureMs: 30_000,
        startedAtUnixUs: nextStartedAtUnixUs
      }
    }
  });
  assert.equal(viewerProjection(state).status, "Capturing");
  state = dispatch(state, { type: "tick", nowUnixUs: nextStartedAtUnixUs + 300_000 });

  const projection = viewerProjection(state);
  assert.equal(projection.status, "Capturing");
  assert.equal(projection.sourceGeneration, 81);
  assert.equal(projection.frameCompletedAtUnixUs, mapping.exposureCompletedAtUnixUs);
  assert.equal(projection.frameAgeMs, 300);

  state = dispatch(state, { type: "tick", nowUnixUs: nextStartedAtUnixUs + 30_500_001 });
  assert.equal(viewerProjection(state).status, "Stale");
});

test("long exposure progress stays Capturing when correlation facts become unknown", () => {
  let state = presentedState();
  const nextStartedAtUnixUs = mapping.exposureCompletedAtUnixUs + 1;
  state = dispatch(state, {
    type: "lifecycle",
    facts: {
      ...ready,
      capture: {
        settingsGeneration: 0,
        exposureMs: 30_000,
        startedAtUnixUs: nextStartedAtUnixUs
      }
    }
  });
  state = dispatch(state, { type: "presented", nowUnixUs: nextStartedAtUnixUs + 300_000 });
  state = dispatch(state, { type: "tick", nowUnixUs: nextStartedAtUnixUs + 1_300_001 });

  const projection = viewerProjection(state);
  assert.equal(projection.status, "Capturing");
  assert.equal(projection.detail, "Exposure in progress · frame freshness unknown");
  assert.equal(projection.sourceGeneration, undefined);
  assert.equal(projection.frameAgeMs, undefined);
});

test("delivery allowance follows measured p99 but stays between 250 and 500 ms", () => {
  let state = readyState();
  state = dispatch(state, { type: "evidence", p99DeliveryUs: 100_000, response: evidence });
  assert.equal(state.deliveryAllowanceUs, 250_000);
  state = dispatch(state, { type: "evidence", p99DeliveryUs: 420_000, response: evidence });
  assert.equal(state.deliveryAllowanceUs, 420_000);
  state = dispatch(state, { type: "evidence", p99DeliveryUs: 900_000, response: evidence });
  assert.equal(state.deliveryAllowanceUs, 500_000);
  assert.equal(state.evidence.response, evidence);
});

test("a new media generation fences cached mappings until a current frame is presented", () => {
  let state = presentedState();
  const previousGeneration = state.mediaConnectionGeneration;

  state = dispatch(state, { type: "media_connecting" });
  state = dispatch(state, { type: "media_connected" });
  assert.equal(state.mediaConnectionGeneration, previousGeneration + 1);
  assert.equal(viewerProjection(state).status, "Reconnecting");
  assert.equal(acceptsMediaPresentation(state, previousGeneration), false);
  assert.equal(acceptsMediaPresentation(state, state.mediaConnectionGeneration), true);

  state = dispatch(state, {
    type: "presented",
    mediaConnectionGeneration: previousGeneration,
    rtpTimestamp: mapping.rtpTimestamp,
    nowUnixUs: mapping.submittedAtUnixUs + 30_000
  });
  assert.equal(viewerProjection(state).status, "Reconnecting");

  state = dispatch(state, { type: "mapping", mapping });
  state = dispatch(state, {
    type: "presented",
    rtpTimestamp: mapping.rtpTimestamp,
    nowUnixUs: mapping.submittedAtUnixUs + 40_000
  });
  assert.equal(viewerProjection(state).status, "Live");
});

test("isolated correlation loss has one second of grace then hides freshness facts", () => {
  let state = presentedState();
  const before = viewerProjection(state);
  state = dispatch(state, {
    type: "presented",
    nowUnixUs: mapping.submittedAtUnixUs + 100_000
  });
  state = dispatch(state, { type: "tick", nowUnixUs: mapping.submittedAtUnixUs + 1_099_999 });
  assert.equal(viewerProjection(state).status, before.status);
  assert.equal(viewerProjection(state).sourceGeneration, 81);

  state = dispatch(state, { type: "tick", nowUnixUs: mapping.submittedAtUnixUs + 1_100_001 });
  const unknown = viewerProjection(state);
  assert.equal(unknown.status, "Stale");
  assert.equal(unknown.detail, "Frame freshness unknown");
  assert.equal(unknown.sourceGeneration, undefined);
  assert.equal(unknown.frameAgeMs, undefined);
  assert.equal(unknown.visibleLatencyMs, undefined);
});

test("confirmed component failure is immediate and retains only a stale trustworthy frame", () => {
  let state = presentedState();
  state = dispatch(state, {
    type: "lifecycle",
    facts: {
      ...ready,
      components: { ...ready.components, encoder: { state: "unavailable", reason: "no_frame" } },
      recovery: "encoder"
    }
  });
  assert.equal(viewerProjection(state).status, "Stale");
  assert.equal(viewerProjection(state).detail, "Recovering encoder");

  state = dispatch(state, {
    type: "lifecycle",
    facts: {
      ...ready,
      components: { ...ready.components, relay: { state: "unavailable", reason: "not_observed" } },
      recovery: "relay"
    }
  });
  assert.equal(viewerProjection(state).status, "Stale");
  assert.equal(viewerProjection(state).detail, "Recovering relay");

  const recovered = reduceViewer(state, { type: "lifecycle", facts: ready });
  state = recovered.state;
  assert.deepEqual(recovered.effects, ["reconnect_media"]);
  assert.equal(viewerProjection(state).status, "Reconnecting");
  const oldConnectionGeneration = state.mediaConnectionGeneration;
  state = dispatch(state, { type: "media_connecting" });
  assert.equal(
    acceptsMediaPresentation(state, oldConnectionGeneration),
    false,
    "the dead pre-recovery WHEP session cannot restore Live"
  );
  state = dispatch(state, { type: "media_connected" });
  state = dispatch(state, { type: "mapping", mapping });
  state = dispatch(state, {
    type: "presented",
    rtpTimestamp: mapping.rtpTimestamp,
    nowUnixUs: mapping.submittedAtUnixUs + 40_000
  });
  assert.equal(viewerProjection(state).status, "Live");

  const withoutFrame = dispatch(readyState(), {
    type: "lifecycle",
    facts: {
      ...ready,
      components: { ...ready.components, capture: { state: "unavailable", reason: "no_camera_source" } },
      recovery: "capture",
      capture: null
    }
  });
  assert.equal(viewerProjection(withoutFrame).status, "Unavailable");
});

test("control transport is independent from live media", () => {
  let state = presentedState();
  state = dispatch(state, { type: "control", event: { type: "connected" } });
  state = dispatch(state, { type: "control", event: { type: "disconnected" } });
  const projection = viewerProjection(state);
  assert.equal(projection.status, "Live");
  assert.equal(projection.controlConnection, "disconnected");
});

test("prior-runtime frames remain stale and can never establish live in the new epoch", () => {
  let state = presentedState();
  state = dispatch(state, {
    type: "lifecycle",
    facts: { ...ready, runtimeEpoch: nextRuntimeEpoch }
  });
  assert.equal(viewerProjection(state).status, "Stale");
  assert.equal(viewerProjection(state).sourceGeneration, undefined);

  state = dispatch(state, {
    type: "presented",
    rtpTimestamp: mapping.rtpTimestamp,
    nowUnixUs: mapping.submittedAtUnixUs + 30_000
  });
  assert.equal(viewerProjection(state).status, "Stale");
});

test("backgrounding suspends liveness and foreground requests immediate reconnection", () => {
  let state = presentedState();
  state = dispatch(state, { type: "visibility", visibility: "hidden" });
  state = dispatch(state, { type: "tick", nowUnixUs: mapping.submittedAtUnixUs + 60_000_000 });
  assert.equal(viewerProjection(state).status, "Live");

  const foreground = reduceViewer(state, { type: "visibility", visibility: "visible" });
  assert.deepEqual(foreground.effects, ["reconnect_media"]);
  assert.equal(viewerProjection(foreground.state).status, "Reconnecting");
});

test("pending settings stay Capturing until that exact generation is visible", () => {
  let state = presentedState();
  const settings = { exposureMs: 20, gain: 350, treatment: "colour" };
  state = dispatch(state, {
    type: "control",
    event: { type: "accepted", targetGeneration: 1, settings }
  });
  state = dispatch(state, {
    type: "control",
    event: { type: "applied", settingsGeneration: 1, settings }
  });
  assert.equal(viewerProjection(state).status, "Capturing");
  assert.equal(state.control.settings.pending?.generation, 1);

  const newMapping = parseFrameMapping({
    ...mapping,
    schemaVersion: 1,
    type: "frame_mapping",
    rtpTimestamp: 43,
    sourceGeneration: 82,
    settingsGeneration: 1,
    treatment: "colour"
  });
  state = dispatch(state, { type: "mapping", mapping: newMapping });
  state = dispatch(state, {
    type: "presented",
    rtpTimestamp: newMapping.rtpTimestamp,
    nowUnixUs: newMapping.submittedAtUnixUs + 20_000
  });
  assert.equal(state.control.settings.pending, null);
  assert.equal(viewerProjection(state).status, "Live");
});
