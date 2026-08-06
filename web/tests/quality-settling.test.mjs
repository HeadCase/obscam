import assert from "node:assert/strict";
import test from "node:test";

import { QualityObservationSettler } from "../dist/quality-settling.js";

function harness() {
  let nextTimer = 0;
  const callbacks = new Map();
  const settled = [];
  const settler = new QualityObservationSettler(
    1_100,
    {
      schedule(callback) {
        nextTimer += 1;
        callbacks.set(nextTimer, callback);
        return nextTimer;
      },
      cancel(timer) {
        callbacks.delete(timer);
      }
    },
    (observation, presentedAtUnixUs) => settled.push({ observation, presentedAtUnixUs })
  );
  const fireNext = () => {
    const [timer, callback] = callbacks.entries().next().value;
    callbacks.delete(timer);
    callback();
  };
  return { settler, callbacks, settled, fireNext };
}

const unknown = {
  correlation: "unknown",
  streamEpoch: 2,
  presentedFrames: 7,
  visibility: "visible"
};

test("late mapping promotion preserves the original presentation timestamp", () => {
  const { settler, settled, fireNext } = harness();
  settler.queue(unknown, 1_700_000_000_000_123, 42);
  assert.equal(settler.promote(42, 3), true);
  fireNext();

  assert.deepEqual(settled, [{
    observation: { ...unknown, correlation: "exact", streamEpoch: 3, rtpTimestamp: 42 },
    presentedAtUnixUs: 1_700_000_000_000_123
  }]);
});

test("replacement and reconnect clearing cancel stale reports without reordering", () => {
  const { settler, callbacks, settled, fireNext } = harness();
  settler.queue(unknown, 100, 42);
  settler.queue({ ...unknown, presentedFrames: 7 }, 200, 43);
  assert.equal(callbacks.size, 1);
  fireNext();
  assert.equal(settled[0].presentedAtUnixUs, 200);

  settler.queue({ ...unknown, presentedFrames: 8 }, 300, 44);
  settler.clear();
  assert.equal(callbacks.size, 0);
  assert.equal(settled.length, 1);
  assert.equal(settler.promote(44, 3), false);
});
