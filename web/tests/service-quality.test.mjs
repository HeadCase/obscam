import assert from "node:assert/strict";
import test from "node:test";

import {
  parseServiceQualityResponse,
  serviceQualityText
} from "../dist/service-quality.js";

const clientId = "8d4cc9fd-b91f-4d0c-a2e3-0f3838608262";
const runtimeEpoch = "4d683f48-cb6e-4bb2-970d-6ee40ca9c118";

function aggregate(overrides = {}) {
  return {
    sampleCount: 2,
    exactCorrelation: 1,
    unknownCorrelation: 1,
    reconnects: 0,
    presentationSkips: 0,
    partitions: [],
    ...overrides
  };
}

function response() {
  const partition = {
    clientId,
    runtimeEpoch,
    streamEpoch: 2,
    connectionGeneration: 1,
    settingsGeneration: 7,
    treatment: "monochrome",
    width: 1920,
    height: 1080,
    visibility: "visible",
    sampleCount: 1,
    exactCorrelation: 1,
    unknownCorrelation: 0,
    uniquePresentedFrames: 2,
    uniquePresentedCadenceHz: 20,
    latencyUs: { min: 100_000, p50: 150_000, p95: 200_000, p99: 200_000, max: 200_000 },
    clockUncertaintyUs: { min: 1_000, p50: 1_000, p95: 2_000, p99: 2_000, max: 2_000 }
  };
  return {
    schemaVersion: 1,
    limits: { clients: 16, samplesPerClient: 512 },
    clients: [
      {
        clientId,
        connectionGeneration: 1,
        samples: [{ correlation: "exact" }, { correlation: "unknown" }],
        ...aggregate({ partitions: [partition] })
      }
    ],
    combined: aggregate({ partitions: [partition] })
  };
}

test("authoritative client evidence is parsed and rendered without new aggregates", () => {
  const evidence = parseServiceQualityResponse(response(), clientId);
  assert.equal(
    serviceQualityText(evidence),
    "2 samples · 1 exact · 1 unknown · 20.0 fps · p95 200 ms"
  );
});

test("the fixed server retention limits are required by the browser contract", () => {
  assert.throws(
    () => parseServiceQualityResponse({ ...response(), limits: { clients: 17, samplesPerClient: 512 } }, clientId),
    /invalid service-quality response/
  );
});

test("client scope and aggregate arithmetic are validated", () => {
  assert.throws(
    () => parseServiceQualityResponse(response(), runtimeEpoch),
    /invalid service-quality client response/
  );
  const invalid = response();
  invalid.clients[0].unknownCorrelation = 2;
  assert.throws(
    () => parseServiceQualityResponse(invalid, clientId),
    /invalid service-quality client response/
  );
});
