import assert from "node:assert/strict";
import test from "node:test";

import { deriveViewerState, parseRuntimeContract } from "../dist/model.js";

const unavailableRuntime = {
  schemaVersion: 1,
  runtimeEpoch: "8d4cc9fd-b91f-4d0c-a2e3-0f3838608262",
  media: { whepPort: 8889, whepPath: "/obscam/whep" },
  components: {
    capture: { state: "unavailable", reason: "no_camera_source" },
    encoder: { state: "unavailable", reason: "no_frame" },
    relay: { state: "unavailable", reason: "not_observed" }
  },
  latestFrame: null
};

test("absent trustworthy frame facts derive Unavailable without invented metrics", () => {
  const runtime = parseRuntimeContract(unavailableRuntime);
  const viewer = deriveViewerState(runtime);

  assert.equal(viewer.status, "Unavailable");
  assert.equal(viewer.frameAgeMs, undefined);
  assert.equal(viewer.captureCadenceHz, undefined);
  assert.equal(viewer.sourceGeneration, undefined);
  assert.equal(viewer.visibleLatencyMs, undefined);
});

test("malformed runtime input fails closed", () => {
  assert.throws(
    () => parseRuntimeContract({ ...unavailableRuntime, schemaVersion: 2 }),
    /unsupported runtime contract/
  );
  assert.throws(
    () => parseRuntimeContract({ ...unavailableRuntime, latestFrame: { sourceGeneration: 12 } }),
    /untrusted frame contract/
  );
  for (const whepPath of [
    "/obscam/whep?token=guess",
    "/obscam/whep#fragment",
    "/obscam/whep path",
    `/${"é".repeat(128)}`
  ]) {
    assert.throws(
      () =>
        parseRuntimeContract({
          ...unavailableRuntime,
          media: { ...unavailableRuntime.media, whepPath }
        }),
      /invalid media descriptor/
    );
  }
});

test("component readiness is parsed independently without inventing a frame", () => {
  const runtime = parseRuntimeContract({
    ...unavailableRuntime,
    components: {
      capture: { state: "ready" },
      encoder: unavailableRuntime.components.encoder,
      relay: { state: "ready" }
    }
  });

  assert.equal(runtime.components.capture.state, "ready");
  assert.equal(runtime.components.encoder.state, "unavailable");
  assert.equal(runtime.components.relay.state, "ready");
  assert.deepEqual(deriveViewerState(runtime), { status: "Unavailable" });
});
