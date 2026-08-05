/** Bounded delay that lets late RTP mappings promote diagnostics without reordering reports. */
export class QualityObservationSettler {
    delayMs;
    scheduler;
    settled;
    pending = new Map();
    byRtpTimestamp = new Map();
    constructor(delayMs, scheduler, settled) {
        this.delayMs = delayMs;
        this.scheduler = scheduler;
        this.settled = settled;
    }
    queue(observation, presentedAtUnixUs, rtpTimestamp) {
        const existing = this.pending.get(observation.presentedFrames);
        if (existing !== undefined)
            this.remove(existing, observation.presentedFrames);
        const timer = this.scheduler.schedule(() => {
            const pending = this.pending.get(observation.presentedFrames);
            if (pending === undefined)
                return;
            this.pending.delete(observation.presentedFrames);
            if (pending.rtpTimestamp !== undefined)
                this.byRtpTimestamp.delete(pending.rtpTimestamp);
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
    promote(rtpTimestamp, streamEpoch) {
        const presentedFrames = this.byRtpTimestamp.get(rtpTimestamp);
        if (presentedFrames === undefined)
            return false;
        const pending = this.pending.get(presentedFrames);
        if (pending === undefined)
            return false;
        pending.observation = {
            ...pending.observation,
            correlation: "exact",
            streamEpoch,
            rtpTimestamp
        };
        return true;
    }
    clear() {
        for (const [presentedFrames, pending] of this.pending) {
            this.remove(pending, presentedFrames);
        }
    }
    remove(pending, presentedFrames) {
        this.scheduler.cancel(pending.timer);
        this.pending.delete(presentedFrames);
        if (pending.rtpTimestamp !== undefined)
            this.byRtpTimestamp.delete(pending.rtpTimestamp);
    }
}
