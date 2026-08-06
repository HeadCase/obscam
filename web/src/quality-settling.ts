import type { QualityObservation } from "./service-quality.js";

interface SettlingScheduler {
  schedule(callback: () => void, delayMs: number): number;
  cancel(timer: number): void;
}

interface PendingObservation {
  observation: QualityObservation;
  presentedAtUnixUs: number;
  rtpTimestamp?: number;
  timer: number;
}

/** Bounded delay that lets late RTP mappings promote diagnostics without reordering reports. */
export class QualityObservationSettler {
  private readonly pending = new Map<number, PendingObservation>();
  private readonly byRtpTimestamp = new Map<number, number>();

  constructor(
    private readonly delayMs: number,
    private readonly scheduler: SettlingScheduler,
    private readonly settled: (observation: QualityObservation, presentedAtUnixUs: number) => void
  ) {}

  queue(
    observation: QualityObservation,
    presentedAtUnixUs: number,
    rtpTimestamp?: number
  ): void {
    const existing = this.pending.get(observation.presentedFrames);
    if (existing !== undefined) this.remove(existing, observation.presentedFrames);
    const timer = this.scheduler.schedule(() => {
      const pending = this.pending.get(observation.presentedFrames);
      if (pending === undefined) return;
      this.pending.delete(observation.presentedFrames);
      if (pending.rtpTimestamp !== undefined) this.byRtpTimestamp.delete(pending.rtpTimestamp);
      this.settled(pending.observation, pending.presentedAtUnixUs);
    }, this.delayMs);
    this.pending.set(observation.presentedFrames, {
      observation,
      presentedAtUnixUs,
      ...(rtpTimestamp === undefined ? {} : { rtpTimestamp }),
      timer
    });
    if (rtpTimestamp !== undefined) {
      this.byRtpTimestamp.set(rtpTimestamp, observation.presentedFrames);
    }
  }

  promote(rtpTimestamp: number, streamEpoch: number): boolean {
    const presentedFrames = this.byRtpTimestamp.get(rtpTimestamp);
    if (presentedFrames === undefined) return false;
    const pending = this.pending.get(presentedFrames);
    if (pending === undefined) return false;
    pending.observation = {
      ...pending.observation,
      correlation: "exact",
      streamEpoch,
      rtpTimestamp
    };
    return true;
  }

  clear(): void {
    for (const [presentedFrames, pending] of this.pending) {
      this.remove(pending, presentedFrames);
    }
  }

  private remove(pending: PendingObservation, presentedFrames: number): void {
    this.scheduler.cancel(pending.timer);
    this.pending.delete(presentedFrames);
    if (pending.rtpTimestamp !== undefined) this.byRtpTimestamp.delete(pending.rtpTimestamp);
  }
}
