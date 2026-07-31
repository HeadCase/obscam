import assert from "node:assert/strict";
import test from "node:test";

import {
  ServiceQualityClient,
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

function response(expectedClientId = clientId) {
  const partition = {
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
        clientId: expectedClientId,
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

test("presentation observations are capped and sent in one bounded batch", async (context) => {
  const originalFetch = globalThis.fetch;
  const originalWindow = globalThis.window;
  context.after(() => {
    globalThis.fetch = originalFetch;
    globalThis.window = originalWindow;
  });
  globalThis.window = { setTimeout };
  const reports = [];
  globalThis.fetch = async (url, options = {}) => {
    if (url === "/api/v1/clock") {
      return responseWithJson({ schemaVersion: 1, serverUnixUs: Date.now() * 1_000 });
    }
    if (url === "/api/v1/service-quality/connections") {
      const request = JSON.parse(options.body);
      return responseWithJson({
        schemaVersion: 1,
        clientId: request.clientId,
        connectionGeneration: 1,
        reconnects: 0
      });
    }
    if (url === "/api/v1/service-quality") {
      reports.push(JSON.parse(options.body));
      return { ok: true, status: 204 };
    }
    if (String(url).startsWith("/api/v1/service-quality?")) {
      const expectedClientId = new URL(String(url), "http://localhost").searchParams.get("clientId");
      return responseWithJson(response(expectedClientId));
    }
    throw new Error(`unexpected request ${url}`);
  };

  const client = await ServiceQualityClient.connect(runtimeEpoch, () => {});
  for (let presentedFrames = 1; presentedFrames <= 33; presentedFrames += 1) {
    client.report({
      correlation: "unknown",
      streamEpoch: null,
      presentedFrames,
      visibility: "visible"
    });
  }
  await new Promise((resolve) => setTimeout(resolve, 350));

  assert.equal(reports.length, 1);
  assert.equal(reports[0].samples.length, 32);
  assert.equal(reports[0].samples[0].presentedFrames, 2);
  assert.equal(reports[0].samples[31].presentedFrames, 33);
});

function responseWithJson(value) {
  return { ok: true, status: 200, json: async () => value };
}

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
