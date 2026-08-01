/** Timer boundary used to test reconnect scheduling without wall-clock delays. */
export interface ReconnectClock {
  setTimeout(callback: () => void, delayMs: number): number;
  clearTimeout(timer: number): void;
}

/** Bounded retry policy and optional browser-boundary hooks for one transport. */
export interface ReconnectOptions {
  initialDelayMs: number;
  maximumDelayMs: number;
  stableAfterMs: number;
  random?: () => number;
  clock?: ReconnectClock;
  onRetryScheduled?: (delayMs: number) => void;
}

const browserClock: ReconnectClock = {
  setTimeout: (callback, delayMs) => window.setTimeout(callback, delayMs),
  clearTimeout: (timer) => window.clearTimeout(timer)
};

/**
 * Owns one transport's retry generation, bounded backoff, and cancellation.
 * Generations fence every asynchronous transport callback; closing the loop
 * advances that fence so in-flight work cannot complete after page teardown.
 */
export class ReconnectLoop {
  private readonly clock: ReconnectClock;
  private readonly random: () => number;
  private generation = 0;
  private failuresSinceStable = 0;
  private retryTimer: number | null = null;
  private stabilityTimer: number | null = null;
  private running = false;
  private scheduledDelayMs: number | null = null;

  constructor(
    private readonly attempt: (generation: number) => void,
    private readonly options: ReconnectOptions
  ) {
    this.clock = options.clock ?? browserClock;
    this.random = options.random ?? Math.random;
  }

  /** The generation of the current attempt, including the close fence. */
  get connectionGeneration(): number {
    return this.generation;
  }

  /** The currently scheduled retry delay, or null during an active attempt. */
  get retryDelayMs(): number | null {
    return this.scheduledDelayMs;
  }

  /** Starts exactly one immediate connection attempt. */
  start(): void {
    if (this.running) return;
    this.running = true;
    this.beginAttempt();
  }

  /** Starts the stability window whose completion resets accumulated backoff. */
  connected(generation: number): void {
    if (!this.isCurrent(generation)) return;
    this.cancelStabilityTimer();
    this.stabilityTimer = this.clock.setTimeout(() => {
      this.stabilityTimer = null;
      if (this.isCurrent(generation)) this.failuresSinceStable = 0;
    }, this.options.stableAfterMs);
  }

  /** Ignores stale failure reports, retries once immediately, then backs off. */
  failed(generation: number): void {
    if (!this.isCurrent(generation)) return;
    this.cancelStabilityTimer();
    const priorFailures = this.failuresSinceStable;
    this.failuresSinceStable += 1;
    if (priorFailures === 0) {
      this.beginAttempt();
      return;
    }
    const baseDelayMs = Math.min(
      this.options.initialDelayMs * (2 ** (priorFailures - 1)),
      this.options.maximumDelayMs
    );
    const random = Math.max(0, Math.min(1, this.random()));
    const delayMs = Math.round(baseDelayMs * (0.5 + random * 0.5));
    this.cancelRetryTimer();
    this.scheduledDelayMs = delayMs;
    this.options.onRetryScheduled?.(delayMs);
    this.retryTimer = this.clock.setTimeout(() => {
      this.retryTimer = null;
      this.beginAttempt();
    }, delayMs);
  }

  /** Cancels accumulated delay without resetting backoff and attempts now. */
  retryNow(): void {
    if (!this.running) return;
    this.cancelRetryTimer();
    this.cancelStabilityTimer();
    this.beginAttempt();
  }

  /** Cancels all timers and invalidates every in-flight generation callback. */
  close(): void {
    this.running = false;
    this.cancelRetryTimer();
    this.cancelStabilityTimer();
    this.generation += 1;
  }

  private beginAttempt(): void {
    if (!this.running) return;
    this.cancelRetryTimer();
    this.scheduledDelayMs = null;
    this.generation += 1;
    this.attempt(this.generation);
  }

  private isCurrent(generation: number): boolean {
    return this.running && generation === this.generation;
  }

  private cancelRetryTimer(): void {
    if (this.retryTimer !== null) {
      this.clock.clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
    this.scheduledDelayMs = null;
  }

  private cancelStabilityTimer(): void {
    if (this.stabilityTimer !== null) {
      this.clock.clearTimeout(this.stabilityTimer);
      this.stabilityTimer = null;
    }
  }
}
