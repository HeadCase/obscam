import assert from "node:assert/strict";
import test from "node:test";

import {
  advanceExposureRail,
  exposureRailProgress,
  initialExposureRailState,
  initialOperatorUiState,
  reduceOperatorUi,
  settingPresentationPhase
} from "../dist/operator-ui.js";

test("genuine activity reveals chrome and restarts the five-second timeout", () => {
  let state = initialOperatorUiState(0);
  state = reduceOperatorUi(state, { type: "tick", nowMs: 5_000 });
  assert.equal(state.chromeVisible, false);

  state = reduceOperatorUi(state, { type: "activity", nowMs: 6_000 });
  assert.equal(state.chromeVisible, true);
  state = reduceOperatorUi(state, { type: "tick", nowMs: 10_999 });
  assert.equal(state.chromeVisible, true);
  state = reduceOperatorUi(state, { type: "tick", nowMs: 11_000 });
  assert.equal(state.chromeVisible, false);
});

test("a bare-feed activation toggles chrome without changing settings state", () => {
  let state = initialOperatorUiState(0);
  state = reduceOperatorUi(state, { type: "chrome_toggled", nowMs: 100 });
  assert.equal(state.chromeVisible, false);
  assert.equal(state.settingsOpen, false);
  state = reduceOperatorUi(state, { type: "chrome_toggled", nowMs: 200 });
  assert.equal(state.chromeVisible, true);
});

test("hover focus press and drag independently protect chrome from hiding", () => {
  let state = initialOperatorUiState(0);
  state = reduceOperatorUi(state, {
    type: "protection_changed",
    protection: "hover",
    active: true,
    nowMs: 1_000
  });
  state = reduceOperatorUi(state, {
    type: "protection_changed",
    protection: "focus",
    active: true,
    nowMs: 1_100
  });
  state = reduceOperatorUi(state, { type: "tick", nowMs: 8_000 });
  assert.equal(state.chromeVisible, true);

  state = reduceOperatorUi(state, {
    type: "protection_changed",
    protection: "hover",
    active: false,
    nowMs: 8_100
  });
  state = reduceOperatorUi(state, { type: "tick", nowMs: 14_000 });
  assert.equal(state.chromeVisible, true, "focus still protects the chrome");

  state = reduceOperatorUi(state, {
    type: "protection_changed",
    protection: "focus",
    active: false,
    nowMs: 14_100
  });
  state = reduceOperatorUi(state, { type: "tick", nowMs: 19_099 });
  assert.equal(state.chromeVisible, true);
  state = reduceOperatorUi(state, { type: "tick", nowMs: 19_100 });
  assert.equal(state.chromeVisible, false);
});

test("taking control opens settings and automatic hiding preserves the draft surface state", () => {
  let state = initialOperatorUiState(0);
  state = reduceOperatorUi(state, { type: "authority_granted", nowMs: 2_000 });
  assert.equal(state.settingsOpen, true);
  assert.equal(state.chromeVisible, true);

  state = reduceOperatorUi(state, { type: "tick", nowMs: 7_000 });
  assert.equal(state.chromeVisible, false);
  assert.equal(state.settingsOpen, false);

  state = reduceOperatorUi(state, { type: "activity", nowMs: 8_000 });
  state = reduceOperatorUi(state, { type: "settings_toggled", nowMs: 8_100 });
  assert.equal(state.settingsOpen, true);
});

test("losing authority closes settings without hiding the displacement announcement", () => {
  let state = initialOperatorUiState(0);
  state = reduceOperatorUi(state, { type: "authority_granted", nowMs: 100 });
  state = reduceOperatorUi(state, { type: "authority_lost", nowMs: 200 });
  assert.equal(state.settingsOpen, false);
  assert.equal(state.chromeVisible, true);
});

test("exact frame presentation produces a bounded arrival acknowledgement", () => {
  let state = initialOperatorUiState(0);
  state = reduceOperatorUi(state, { type: "exact_frame_presented", nowMs: 1_000 });
  assert.equal(state.frameArrivalUntilMs, 1_300);
  state = reduceOperatorUi(state, { type: "tick", nowMs: 1_299 });
  assert.equal(state.frameArrivalUntilMs, 1_300);
  state = reduceOperatorUi(state, { type: "tick", nowMs: 1_300 });
  assert.equal(state.frameArrivalUntilMs, null);
});

test("setting rejection produces a brief browser-local acknowledgement", () => {
  let state = initialOperatorUiState(0);
  state = reduceOperatorUi(state, { type: "setting_rejected", nowMs: 1_000 });
  assert.equal(state.settingRejectionUntilMs, 1_650);
  state = reduceOperatorUi(state, { type: "tick", nowMs: 1_649 });
  assert.equal(state.settingRejectionUntilMs, 1_650);
  state = reduceOperatorUi(state, { type: "tick", nowMs: 1_650 });
  assert.equal(state.settingRejectionUntilMs, null);
});

test("exposure rail begins at one second and holds at completion", () => {
  assert.equal(exposureRailProgress(50, 10_000, 10_025), null);
  assert.equal(exposureRailProgress(1_000, 10_000, 10_000), 0);
  assert.equal(exposureRailProgress(1_000, 10_000, 10_500), 0.5);
  assert.equal(exposureRailProgress(1_000, 10_000, 11_500), 1);
});

test("exposure rail never regresses within one capture but resets for the next capture", () => {
  let state = initialExposureRailState();
  state = advanceExposureRail(state, {
    exposureMs: 1_000,
    startedAtUnixUs: 10_000_000,
    nowUnixUs: 10_600_000
  });
  assert.equal(state.progress, 0.6);

  state = advanceExposureRail(state, {
    exposureMs: 1_000,
    startedAtUnixUs: 10_000_000,
    nowUnixUs: 10_200_000
  });
  assert.equal(state.progress, 0.6, "an unrelated stale render cannot rewind the rail");

  state = advanceExposureRail(state, {
    exposureMs: 1_000,
    startedAtUnixUs: 11_000_000,
    nowUnixUs: 11_000_000
  });
  assert.equal(state.progress, 0, "a new authoritative capture starts a new rail cycle");
});

test("short exposures suppress the rail without retaining a stale capture identity", () => {
  const state = advanceExposureRail(initialExposureRailState(), {
    exposureMs: 500,
    startedAtUnixUs: 10_000_000,
    nowUnixUs: 10_250_000
  });
  assert.deepEqual(state, initialExposureRailState());
});

test("settings remain unknown until exact presentation evidence exists", () => {
  const base = {
    visibleKnown: false,
    drafted: false,
    requestedDiffers: true,
    rejected: false,
    applied: false,
    awaitingPresentation: false
  };
  assert.equal(settingPresentationPhase(base), "unknown");
  assert.equal(
    settingPresentationPhase({ ...base, awaitingPresentation: true }),
    "requested"
  );
  assert.equal(
    settingPresentationPhase({ ...base, awaitingPresentation: true, applied: true }),
    "applied"
  );
  assert.equal(
    settingPresentationPhase({ ...base, visibleKnown: true, requestedDiffers: false }),
    "visible"
  );
});
