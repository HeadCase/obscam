import assert from "node:assert/strict";
import test from "node:test";

import {
  ServiceQualityClient,
  fetchServiceQualityReport,
  parseServiceQualityResponse,
  parseServiceQualitySummary,
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

function summary(expectedClientId = clientId) {
  const value = structuredClone(response(expectedClientId));
  for (const client of value.clients) delete client.samples;
  return value;
}

test("the banner renders the current partition instead of historical startup unknowns", () => {
  const evidence = parseServiceQualitySummary(summary(), clientId);
  assert.equal(
    serviceQualityText(evidence),
    "1 samples · 1 exact · 0 unknown · 20.0 fps · p95 200 ms"
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
    if (String(url).startsWith("/api/v1/service-quality/summary?")) {
      const expectedClientId = new URL(String(url), "http://localhost").searchParams.get("clientId");
      return responseWithJson(summary(expectedClientId));
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
    }, presentedFrames === 33 ? 1_700_000_000_000_123 : undefined);
  }
  await new Promise((resolve) => setTimeout(resolve, 350));

  assert.equal(reports.length, 1);
  assert.equal(reports[0].samples.length, 32);
  assert.equal(reports[0].samples[0].presentedFrames, 2);
  assert.equal(reports[0].samples[31].presentedFrames, 33);
  assert.equal(reports[0].samples[31].presentedAtUnixUs, 1_700_000_000_000_123);
});

test("a media reconnect discards queued observations from the previous track", async (context) => {
  const originalFetch = globalThis.fetch;
  const originalWindow = globalThis.window;
  context.after(() => {
    globalThis.fetch = originalFetch;
    globalThis.window = originalWindow;
  });
  globalThis.window = { setTimeout, clearTimeout };
  let connectionGeneration = 0;
  const reports = [];
  globalThis.fetch = async (url, options = {}) => {
    if (url === "/api/v1/clock") {
      return responseWithJson({ schemaVersion: 1, serverUnixUs: Date.now() * 1_000 });
    }
    if (url === "/api/v1/service-quality/connections") {
      const request = JSON.parse(options.body);
      connectionGeneration += 1;
      return responseWithJson({
        schemaVersion: 1,
        clientId: request.clientId,
        connectionGeneration,
        reconnects: connectionGeneration - 1
      });
    }
    if (url === "/api/v1/service-quality") {
      reports.push(JSON.parse(options.body));
      return { ok: true, status: 204 };
    }
    if (String(url).startsWith("/api/v1/service-quality/summary?")) {
      const expectedClientId = new URL(String(url), "http://localhost").searchParams.get("clientId");
      const value = summary(expectedClientId);
      value.clients[0].connectionGeneration = connectionGeneration;
      return responseWithJson(value);
    }
    throw new Error(`unexpected request ${url}`);
  };

  const client = await ServiceQualityClient.connect(runtimeEpoch, () => {});
  client.report({
    correlation: "unknown",
    streamEpoch: 2,
    presentedFrames: 3_493,
    visibility: "visible"
  });
  await client.reconnect();
  client.report({
    correlation: "unknown",
    streamEpoch: 3,
    presentedFrames: 1,
    visibility: "visible"
  });
  await new Promise((resolve) => setTimeout(resolve, 350));

  assert.equal(reports.length, 1);
  assert.equal(reports[0].connectionGeneration, 2);
  assert.deepEqual(reports[0].samples.map((sample) => sample.presentedFrames), [1]);
});

test("a media reconnect fences an in-flight old-generation report", async (context) => {
  const originalFetch = globalThis.fetch;
  const originalWindow = globalThis.window;
  context.after(() => {
    globalThis.fetch = originalFetch;
    globalThis.window = originalWindow;
  });
  globalThis.window = { setTimeout, clearTimeout };
  let connectionGeneration = 0;
  let releaseOldReport;
  let oldReportStarted;
  const oldReportIsStarted = new Promise((resolve) => { oldReportStarted = resolve; });
  const oldReportMayFinish = new Promise((resolve) => { releaseOldReport = resolve; });
  const reports = [];
  globalThis.fetch = async (url, options = {}) => {
    if (url === "/api/v1/clock") {
      return responseWithJson({ schemaVersion: 1, serverUnixUs: Date.now() * 1_000 });
    }
    if (url === "/api/v1/service-quality/connections") {
      const request = JSON.parse(options.body);
      connectionGeneration += 1;
      return responseWithJson({
        schemaVersion: 1,
        clientId: request.clientId,
        connectionGeneration,
        reconnects: connectionGeneration - 1
      });
    }
    if (url === "/api/v1/service-quality") {
      reports.push(JSON.parse(options.body));
      if (reports.length === 1) {
        oldReportStarted();
        await oldReportMayFinish;
      }
      return { ok: true, status: 204 };
    }
    if (String(url).startsWith("/api/v1/service-quality/summary?")) {
      const expectedClientId = new URL(String(url), "http://localhost").searchParams.get("clientId");
      const value = summary(expectedClientId);
      value.clients[0].connectionGeneration = connectionGeneration;
      return responseWithJson(value);
    }
    throw new Error(`unexpected request ${url}`);
  };

  const client = await ServiceQualityClient.connect(runtimeEpoch, () => {});
  client.report({
    correlation: "unknown",
    streamEpoch: 2,
    presentedFrames: 3_493,
    visibility: "visible"
  });
  await oldReportIsStarted;
  const reconnect = client.reconnect();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(connectionGeneration, 1, "server generation waits for the old report");
  client.report({
    correlation: "unknown",
    streamEpoch: 3,
    presentedFrames: 1,
    visibility: "visible"
  });
  releaseOldReport();
  await reconnect;
  client.report({
    correlation: "unknown",
    streamEpoch: 3,
    presentedFrames: 1,
    visibility: "visible"
  });
  await new Promise((resolve) => setTimeout(resolve, 350));

  assert.equal(reports.length, 2);
  assert.deepEqual(
    reports.map((report) => ({
      generation: report.connectionGeneration,
      frames: report.samples.map((sample) => sample.presentedFrames)
    })),
    [
      { generation: 1, frames: [3_493] },
      { generation: 2, frames: [1] }
    ]
  );
});

test("full retained evidence is fetched only on demand", async (context) => {
  const originalFetch = globalThis.fetch;
  context.after(() => {
    globalThis.fetch = originalFetch;
  });
  const requests = [];
  globalThis.fetch = async (url) => {
    requests.push(String(url));
    return responseWithJson(response(clientId));
  };

  const evidence = await fetchServiceQualityReport(clientId);

  assert.equal(evidence.clients[0].samples.length, 2);
  assert.deepEqual(requests, [`/api/v1/service-quality?clientId=${clientId}`]);
});

test("viewer timestamps use the calibrated Rust service clock", async (context) => {
  const originalFetch = globalThis.fetch;
  const originalNow = Date.now;
  context.after(() => {
    globalThis.fetch = originalFetch;
    Date.now = originalNow;
  });
  let localNowMs = 1_000;
  let clockRequests = 0;
  Date.now = () => localNowMs;
  globalThis.fetch = async (url, options = {}) => {
    if (url === "/api/v1/clock") {
      const delaysMs = [20, 2, 10];
      const offsetsUs = [1_000_000, 2_000_000, 3_000_000];
      const delayMs = delaysMs[clockRequests++];
      const startedAtMs = localNowMs;
      localNowMs += delayMs;
      return responseWithJson({
        schemaVersion: 1,
        serverUnixUs: (startedAtMs + delayMs / 2) * 1_000 + offsetsUs[clockRequests - 1]
      });
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
    throw new Error(`unexpected request ${url}`);
  };

  const client = await ServiceQualityClient.connect(runtimeEpoch, () => {});
  localNowMs = 1_100;
  assert.equal(clockRequests, 3);
  assert.equal(client.nowUnixUs(), 3_100_000);
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
