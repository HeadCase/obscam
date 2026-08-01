import assert from "node:assert/strict";
import test from "node:test";

import { ReconnectLoop } from "../dist/reconnect.js";

class ManualClock {
  now = 0;
  nextId = 1;
  timers = new Map();

  setTimeout(callback, delayMs) {
    const id = this.nextId++;
    this.timers.set(id, { at: this.now + delayMs, callback });
    return id;
  }

  clearTimeout(id) {
    this.timers.delete(id);
  }

  advance(delayMs) {
    const target = this.now + delayMs;
    while (true) {
      const next = [...this.timers.entries()]
        .filter(([, timer]) => timer.at <= target)
        .sort((left, right) => left[1].at - right[1].at || left[0] - right[0])[0];
      if (next === undefined) break;
      const [id, timer] = next;
      this.timers.delete(id);
      this.now = timer.at;
      timer.callback();
    }
    this.now = target;
  }
}

test("transport retries immediately, backs off independently, and resets after stability", () => {
  const clock = new ManualClock();
  const attempts = [];
  const loop = new ReconnectLoop(
    (generation) => attempts.push({ generation, at: clock.now }),
    {
      initialDelayMs: 200,
      maximumDelayMs: 1_000,
      stableAfterMs: 500,
      random: () => 0.5,
      clock
    }
  );

  loop.start();
  assert.deepEqual(attempts, [{ generation: 1, at: 0 }]);

  loop.failed(1);
  assert.deepEqual(attempts.at(-1), { generation: 2, at: 0 });

  loop.failed(1);
  assert.equal(attempts.length, 2, "a stale generation cannot schedule a retry");
  loop.failed(2);
  assert.equal(loop.retryDelayMs, 150);
  clock.advance(149);
  assert.equal(attempts.length, 2);
  clock.advance(1);
  assert.deepEqual(attempts.at(-1), { generation: 3, at: 150 });

  loop.failed(3);
  assert.equal(loop.retryDelayMs, 300);
  clock.advance(300);
  assert.deepEqual(attempts.at(-1), { generation: 4, at: 450 });

  loop.failed(4);
  assert.equal(loop.retryDelayMs, 600);
  clock.advance(600);
  loop.failed(5);
  assert.equal(loop.retryDelayMs, 750);
  clock.advance(750);
  loop.failed(6);
  assert.equal(loop.retryDelayMs, 750, "jittered delay stays capped by the maximum");
  clock.advance(750);

  loop.connected(7);
  clock.advance(500);
  loop.failed(7);
  assert.deepEqual(attempts.at(-1), { generation: 8, at: 3_050 });
});

test("forced retry bypasses delay and close cancels every delayed attempt", () => {
  const clock = new ManualClock();
  const attempts = [];
  const loop = new ReconnectLoop(
    (generation) => attempts.push({ generation, at: clock.now }),
    {
      initialDelayMs: 200,
      maximumDelayMs: 1_000,
      stableAfterMs: 500,
      random: () => 0.5,
      clock
    }
  );

  loop.start();
  loop.failed(1);
  loop.failed(2);
  assert.equal(loop.retryDelayMs, 150);

  loop.retryNow();
  assert.deepEqual(attempts.at(-1), { generation: 3, at: 0 });
  clock.advance(1_000);
  assert.equal(attempts.length, 3, "the bypassed delayed retry stays cancelled");

  loop.failed(3);
  loop.close();
  assert.equal(loop.connectionGeneration, 4);
  loop.failed(3);
  clock.advance(10_000);
  assert.equal(attempts.length, 3);
  assert.equal(loop.retryDelayMs, null);
});
