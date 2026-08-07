import assert from "node:assert/strict";
import test from "node:test";

import { isSnapshotCancellation, snapshotFilename } from "../dist/snapshot.js";

test("an exact snapshot filename identifies capture time and source generation", () => {
  assert.equal(
    snapshotFilename(
      { exposureCompletedAtUnixUs: 1_700_000_000_123_000, sourceGeneration: 81 },
      new Date("2026-08-06T12:34:56.789Z")
    ),
    "obscam-captured-20231114T221320.123Z-generation-81.png"
  );
});

test("an unknown snapshot filename labels correlation unknown and identifies save time", () => {
  assert.equal(
    snapshotFilename(null, new Date("2026-08-06T12:34:56.789Z")),
    "obscam-saved-20260806T123456.789Z-correlation-unknown.png"
  );
});

test("only an AbortError is treated as Save As cancellation", () => {
  assert.equal(isSnapshotCancellation(new DOMException("cancelled", "AbortError")), true);
  assert.equal(isSnapshotCancellation(new Error("disk full")), false);
});
