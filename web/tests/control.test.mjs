import assert from "node:assert/strict";
import test from "node:test";

import {
  initialControlState,
  parseControlMessage,
  reduceControl
} from "../dist/control.js";

const credentials = {
  runtimeEpoch: "8d4cc9fd-b91f-4d0c-a2e3-0f3838608262",
  generation: 4,
  secret: "ab".repeat(32)
};

test("a new viewer starts read-only and control loss disables mutation", () => {
  const initial = initialControlState(null);
  assert.equal(initial.connection, "disconnected");
  assert.equal(initial.connectionGeneration, 0);
  assert.equal(initial.retryDelayMs, null);
  assert.equal(initial.ownership, "no_one");
  assert.equal(initial.mayMutate, false);

  const connecting = reduceControl(initial, {
    type: "connecting",
    connectionGeneration: 1
  }).state;
  const connected = reduceControl(connecting, {
    type: "connected",
    connectionGeneration: 1
  }).state;
  assert.equal(connected.mayMutate, false);
  const granted = reduceControl(connected, {
    type: "granted",
    credentials
  }).state;
  assert.equal(granted.ownership, "you");
  assert.equal(granted.mayMutate, true);

  const disconnected = reduceControl(granted, {
    type: "disconnected",
    connectionGeneration: 1
  }).state;
  assert.equal(disconnected.connection, "disconnected");
  assert.equal(disconnected.mayMutate, false);
  assert.deepEqual(disconnected.credentials, credentials);
  const waiting = reduceControl(disconnected, {
    type: "retry_scheduled",
    connectionGeneration: 1,
    retryDelayMs: 375
  }).state;
  assert.equal(waiting.retryDelayMs, 375);
  const nextConnecting = reduceControl(waiting, {
    type: "connecting",
    connectionGeneration: 2
  }).state;
  const reconnected = reduceControl(nextConnecting, {
    type: "connected",
    connectionGeneration: 2
  }).state;
  const awaitingResume = reduceControl(reconnected, {
    type: "authority",
    state: "held",
    generation: credentials.generation
  }).state;
  assert.equal(awaitingResume.mayMutate, false);
  assert.equal(
    reduceControl(awaitingResume, { type: "resumed", generation: credentials.generation }).state
      .mayMutate,
    true
  );
});

test("stale control connection callbacks cannot restore authority", () => {
  let state = initialControlState(credentials);
  state = reduceControl(state, { type: "connecting", connectionGeneration: 2 }).state;
  state = reduceControl(state, { type: "connecting", connectionGeneration: 3 }).state;

  const stale = reduceControl(state, { type: "connected", connectionGeneration: 2 }).state;
  assert.equal(stale.connection, "disconnected");
  assert.equal(stale.connectionGeneration, 3);
  assert.equal(stale.mayMutate, false);
});

test("takeover discards displaced credentials and unsent intent", () => {
  let state = reduceControl(initialControlState(null), { type: "connected", connectionGeneration: 0 }).state;
  state = reduceControl(state, { type: "granted", credentials }).state;
  state = reduceControl(state, { type: "intent_queued" }).state;

  const displaced = reduceControl(state, {
    type: "authority",
    state: "held",
    generation: 5
  });

  assert.equal(displaced.state.ownership, "another_viewer");
  assert.equal(displaced.state.pendingIntent, false);
  assert.equal(displaced.state.credentials, null);
  assert.equal(displaced.storage, "remove");
});

test("server-timed expiry discards the former holder credential", () => {
  let state = reduceControl(initialControlState(null), { type: "connected", connectionGeneration: 0 }).state;
  state = reduceControl(state, { type: "granted", credentials }).state;

  const expired = reduceControl(state, {
    type: "authority",
    state: "unheld",
    generation: credentials.generation
  });

  assert.equal(expired.state.ownership, "no_one");
  assert.equal(expired.state.credentials, null);
  assert.equal(expired.storage, "remove");
});

test("late stale-command rejection does not overwrite newer authoritative ownership", () => {
  let state = reduceControl(initialControlState(null), { type: "connected", connectionGeneration: 0 }).state;
  state = reduceControl(state, { type: "granted", credentials }).state;
  state = reduceControl(state, {
    type: "authority",
    state: "held",
    generation: credentials.generation + 1
  }).state;

  assert.equal(reduceControl(state, { type: "rejected" }).state.ownership, "another_viewer");
});

test("same-runtime credentials are retained for reconnect and other-runtime credentials are not", () => {
  assert.deepEqual(initialControlState(credentials).credentials, credentials);
  assert.equal(
    initialControlState({ ...credentials, runtimeEpoch: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa" }, credentials.runtimeEpoch)
      .credentials,
    null
  );
});

test("server messages validate schema, generations, and secrets", () => {
  assert.deepEqual(
    parseControlMessage({
      schemaVersion: 1,
      type: "authority",
      state: "held",
      generation: 7
    }),
    { type: "authority", state: "held", generation: 7 }
  );
  assert.throws(
    () => parseControlMessage({ schemaVersion: 2, type: "authority", state: "held", generation: 7 }),
    /unsupported control message/
  );
  assert.throws(
    () =>
      parseControlMessage({
        schemaVersion: 1,
        type: "renewed",
        generation: 7,
        leaseDurationMs: 4_999
      }),
    /invalid control message/
  );
  assert.throws(
    () => parseControlMessage({ schemaVersion: 1, type: "granted", generation: 7, secret: "short" }),
    /invalid control message/
  );
});

test("settings messages preserve complete accepted and applied tuples", () => {
  const settings = { exposureMs: 50, gain: 350, treatment: "colour" };
  assert.deepEqual(
    parseControlMessage({
      schemaVersion: 1,
      type: "settings",
      state: {
        applied: { generation: 0, settings: { exposureMs: 500, gain: 100, treatment: "monochrome" } },
        pending: null
      }
    }).state.applied.settings,
    { exposureMs: 500, gain: 100, treatment: "monochrome" }
  );

  let state = reduceControl(initialControlState(null), {
    type: "accepted",
    targetGeneration: 1,
    settings
  }).state;
  assert.equal(state.pendingIntent, false);
  assert.deepEqual(state.settings.pending, { generation: 1, settings });

  state = reduceControl(state, {
    type: "applied",
    settingsGeneration: 1,
    settings
  }).state;
  assert.deepEqual(state.settings.applied, { generation: 1, settings });
  assert.deepEqual(state.settings.pending, { generation: 1, settings });

  state = reduceControl(state, {
    type: "visible",
    settingsGeneration: 1
  }).state;
  assert.equal(state.settings.pending, null);
  assert.deepEqual(state.settings.visible, { generation: 1, settings });
});

test("settings messages reject exposures below the 50 ms floor", () => {
  assert.throws(
    () => parseControlMessage({
      schemaVersion: 1,
      type: "accepted",
      targetGeneration: 1,
      settings: { exposureMs: 20, gain: 100, treatment: "monochrome" }
    }),
    /invalid control message/
  );
});

test("camera edits remain a local draft until Apply is queued", () => {
  const applied = { exposureMs: 500, gain: 100, treatment: "monochrome" };
  const draft = { exposureMs: 50, gain: 350, treatment: "colour" };
  let state = reduceControl(initialControlState(null), {
    type: "connected",
    connectionGeneration: 0
  }).state;
  state = reduceControl(state, { type: "granted", credentials }).state;

  state = reduceControl(state, { type: "draft_changed", settings: draft }).state;
  assert.deepEqual(state.draft, draft);
  assert.deepEqual(state.settings.applied.settings, applied);
  assert.equal(state.settings.pending, null);
  assert.equal(state.pendingIntent, false);

  state = reduceControl(state, { type: "intent_queued", settings: draft }).state;
  assert.equal(state.pendingIntent, true);
  assert.equal(state.draft, null);
  assert.deepEqual(state.submitted, draft);
  assert.equal(state.notice, "Settings submitted");
  const nextDraft = { ...draft, exposureMs: 100 };
  state = reduceControl(state, { type: "draft_changed", settings: nextDraft }).state;
  assert.deepEqual(state.draft, nextDraft);
  state = reduceControl(state, {
    type: "accepted",
    targetGeneration: 1,
    settings: draft
  }).state;
  assert.deepEqual(state.draft, nextDraft);
  assert.equal(state.submitted, null);
  assert.deepEqual(state.settings.pending, { generation: 1, settings: draft });

  const superseding = { ...draft, exposureMs: 200 };
  state = reduceControl(state, { type: "draft_changed", settings: superseding }).state;
  assert.deepEqual(state.draft, superseding);
});

test("Discard restores the authoritative tuple and authority loss removes a draft", () => {
  const draft = { exposureMs: 50, gain: 350, treatment: "colour" };
  let state = reduceControl(initialControlState(null), {
    type: "connected",
    connectionGeneration: 0
  }).state;
  state = reduceControl(state, { type: "granted", credentials }).state;
  state = reduceControl(state, { type: "draft_changed", settings: draft }).state;
  state = reduceControl(state, { type: "draft_discarded" }).state;
  assert.equal(state.draft, null);

  state = reduceControl(state, { type: "draft_changed", settings: draft }).state;
  state = reduceControl(state, {
    type: "authority",
    state: "held",
    generation: credentials.generation + 1
  }).state;
  assert.equal(state.draft, null);
});

test("presentation before applied still advances the exact target to visible", () => {
  const settings = { exposureMs: 50, gain: 350, treatment: "colour" };
  let state = reduceControl(initialControlState(null), {
    type: "accepted",
    targetGeneration: 1,
    settings
  }).state;

  state = reduceControl(state, { type: "visible", settingsGeneration: 1 }).state;
  assert.equal(state.settings.pending?.generation, 1);
  assert.equal(state.settings.presentedGeneration, 1);

  state = reduceControl(state, {
    type: "applied",
    settingsGeneration: 1,
    settings
  }).state;
  assert.equal(state.settings.pending, null);
  assert.equal(state.settings.presentedGeneration, null);
  assert.deepEqual(state.settings.visible, { generation: 1, settings });
});

test("the initially applied generation becomes independently visible", () => {
  let state = initialControlState(null);

  state = reduceControl(state, { type: "visible", settingsGeneration: 0 }).state;

  assert.deepEqual(state.settings.visible, state.settings.applied);
});

test("invalid settings rejection keeps a healthy lease", () => {
  let state = reduceControl(initialControlState(null), { type: "connected", connectionGeneration: 0 }).state;
  state = reduceControl(state, { type: "granted", credentials }).state;
  const rejectedSettings = { exposureMs: 50, gain: 350, treatment: "colour" };
  state = reduceControl(state, { type: "intent_queued", settings: rejectedSettings }).state;

  const rejected = reduceControl(state, { type: "rejected", reason: "invalid_settings" });
  assert.equal(rejected.state.ownership, "you");
  assert.equal(rejected.state.mayMutate, true);
  assert.deepEqual(rejected.state.credentials, credentials);
  assert.equal(rejected.state.pendingIntent, false);
  assert.deepEqual(rejected.state.rejected, rejectedSettings);
  assert.equal(rejected.state.draft, null);
});

test("reconnect discards unsafe unsent intent and announces the restoration", () => {
  const draft = { exposureMs: 50, gain: 350, treatment: "colour" };
  let state = reduceControl(initialControlState(null), { type: "connected", connectionGeneration: 0 }).state;
  state = reduceControl(state, { type: "granted", credentials }).state;
  state = reduceControl(state, { type: "draft_changed", settings: draft }).state;

  state = reduceControl(state, { type: "connecting", connectionGeneration: 1 }).state;

  assert.equal(state.draft, null);
  assert.equal(state.submitted, null);
  assert.equal(state.notice, "Draft discarded: control reconnecting");
});

test("camera recovery rejection is a valid non-authority failure", () => {
  assert.deepEqual(
    parseControlMessage({
      schemaVersion: 1,
      type: "rejected",
      reason: "camera_unavailable"
    }),
    { type: "rejected", reason: "camera_unavailable" }
  );
});
