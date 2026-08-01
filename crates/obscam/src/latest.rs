use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::{MonochromeFrame, correlation::CapturedFrameMetadata};
use zwo_asi::{HEIGHT, WIDTH};

const I420_BYTES: usize = WIDTH * HEIGHT * 3 / 2;

/// A two-buffer, single-consumer I420 handoff with at most one pending generation.
pub struct LatestFrameMailbox {
    inner: LatestBufferMailbox,
}

impl LatestFrameMailbox {
    /// Allocates one in-flight and one replaceable pending I420 buffer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: LatestBufferMailbox::new(I420_BYTES),
        }
    }

    pub(crate) fn with_epoch_fence(epoch_fence: EpochFence) -> Self {
        Self {
            inner: LatestBufferMailbox::with_epoch_fence(I420_BYTES, epoch_fence),
        }
    }

    /// Copies a complete frame into the sole pending slot, replacing older work.
    ///
    /// Returns one when an older pending frame was replaced, otherwise zero.
    ///
    /// # Panics
    ///
    /// Panics after mailbox mutex poisoning or violation of the single-consumer
    /// buffer-ownership invariant; either condition is an internal invariant failure.
    pub fn publish(&self, frame: &MonochromeFrame<'_>) -> u64 {
        self.inner.publish_current(frame.generation(), frame.data())
    }

    pub(crate) fn publish_in_epoch(&self, epoch: MediaEpoch, frame: &MonochromeFrame<'_>) -> u64 {
        self.inner.publish(epoch, frame.generation(), frame.data())
    }

    pub(crate) fn publish_captured(
        &self,
        epoch: MediaEpoch,
        frame: &MonochromeFrame<'_>,
        metadata: CapturedFrameMetadata,
    ) -> u64 {
        self.inner
            .publish_with_metadata(epoch, frame.generation(), frame.data(), Some(metadata))
    }

    pub(crate) fn commit_owned_if_current<T>(
        &self,
        frame: PublishedFrame,
        commit: impl FnOnce(PublishedFrame) -> T,
    ) -> Result<T, PublishedFrame> {
        if self.inner.is_current(frame.0.identity) {
            Ok(commit(frame))
        } else {
            Err(frame)
        }
    }

    /// Takes the newest pending frame, leaving no queued work.
    ///
    /// # Panics
    ///
    /// Panics after mailbox mutex poisoning, which indicates an internal invariant failure.
    #[must_use]
    pub fn take(&self) -> Option<PublishedFrame> {
        self.inner.take().map(PublishedFrame)
    }

    /// Waits for and takes the newest pending frame, bounded by `timeout`.
    ///
    /// # Panics
    ///
    /// Panics after mailbox mutex poisoning, which indicates an internal invariant failure.
    #[must_use]
    pub fn wait_take(&self, timeout: Duration) -> Option<PublishedFrame> {
        self.inner.wait_take(timeout).map(PublishedFrame)
    }

    /// Returns a completed or discarded in-flight buffer for reuse.
    ///
    /// # Panics
    ///
    /// Panics after mailbox mutex poisoning, which indicates an internal invariant failure.
    pub fn recycle(&self, frame: PublishedFrame) {
        self.inner.recycle(frame.0);
    }

    /// Starts a new semantic media epoch and fences older pending and uncommitted output.
    ///
    /// Same-epoch source generations do not call this method: they replace only
    /// pending work. Settings, treatment, stream, and runtime changes do. The
    /// returned count is one when pending work was discarded, otherwise zero.
    pub fn begin_new_epoch(&self) -> u64 {
        self.inner.begin_new_epoch()
    }

    pub(crate) fn discard_pending(&self) -> u64 {
        self.inner.discard_pending()
    }

    /// Reports whether claimed output still belongs to the current semantic epoch.
    #[must_use]
    pub fn is_current(&self, frame: &PublishedFrame) -> bool {
        self.inner.is_current(frame.0.identity)
    }
}

impl Default for LatestFrameMailbox {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) struct LatestBufferMailbox {
    state: Mutex<State>,
    available: Condvar,
    epoch_fence: EpochFence,
}

impl LatestBufferMailbox {
    pub(crate) fn new(buffer_bytes: usize) -> Self {
        Self::with_epoch_fence(buffer_bytes, EpochFence::new())
    }

    pub(crate) fn with_epoch_fence(buffer_bytes: usize, epoch_fence: EpochFence) -> Self {
        Self {
            state: Mutex::new(State {
                pending: None,
                spare: vec![new_buffer(buffer_bytes), new_buffer(buffer_bytes)],
                newest: None,
            }),
            available: Condvar::new(),
            epoch_fence,
        }
    }

    pub(crate) fn current_epoch(&self) -> MediaEpoch {
        self.epoch_fence.current()
    }

    pub(crate) fn publish_current(&self, generation: u64, data: &[u8]) -> u64 {
        self.publish(self.current_epoch(), generation, data)
    }

    pub(crate) fn publish(&self, epoch: MediaEpoch, generation: u64, data: &[u8]) -> u64 {
        self.publish_with_metadata(epoch, generation, data, None)
    }

    pub(crate) fn publish_with_metadata(
        &self,
        epoch: MediaEpoch,
        generation: u64,
        data: &[u8],
        metadata: Option<CapturedFrameMetadata>,
    ) -> u64 {
        let identity = FrameIdentity { epoch, generation };
        if !self.epoch_fence.is_current(epoch) {
            return 0;
        }

        let mut state = self.state.lock().expect("frame mailbox mutex poisoned");
        if !self.epoch_fence.is_current(epoch)
            || state.newest.is_some_and(|newest| identity <= newest)
        {
            return 0;
        }

        let skipped = u64::from(state.pending.is_some());
        let mut pending = state.pending.take().unwrap_or_else(|| {
            state
                .spare
                .pop()
                .expect("one buffer remains while the consumer owns at most one")
        });
        pending.identity = identity;
        pending.metadata = metadata;
        pending.data.copy_from_slice(data);
        state.pending = Some(pending);
        state.newest = Some(identity);
        self.available.notify_one();
        skipped
    }

    pub(crate) fn take(&self) -> Option<BufferGeneration> {
        let mut state = self.state.lock().expect("frame mailbox mutex poisoned");
        self.claim_pending(&mut state)
    }

    pub(crate) fn wait_take(&self, timeout: Duration) -> Option<BufferGeneration> {
        let state = self.state.lock().expect("frame mailbox mutex poisoned");
        let (mut state, _) = self
            .available
            .wait_timeout_while(state, timeout, |state| state.pending.is_none())
            .expect("frame mailbox mutex poisoned while waiting");
        self.claim_pending(&mut state)
    }

    fn claim_pending(&self, state: &mut State) -> Option<BufferGeneration> {
        let pending = state.pending.take()?;
        if self.is_current(pending.identity) {
            Some(pending)
        } else {
            state.spare.push(pending);
            None
        }
    }

    pub(crate) fn recycle(&self, frame: BufferGeneration) {
        self.state
            .lock()
            .expect("frame mailbox mutex poisoned")
            .spare
            .push(frame);
    }

    pub(crate) fn begin_new_epoch(&self) -> u64 {
        let mut state = self.state.lock().expect("frame mailbox mutex poisoned");
        self.epoch_fence.advance();
        Self::discard_pending_locked(&mut state)
    }

    pub(crate) fn discard_pending(&self) -> u64 {
        let mut state = self.state.lock().expect("frame mailbox mutex poisoned");
        Self::discard_pending_locked(&mut state)
    }

    fn discard_pending_locked(state: &mut State) -> u64 {
        let discarded = u64::from(state.pending.is_some());
        if let Some(pending) = state.pending.take() {
            state.spare.push(pending);
        }
        discarded
    }

    pub(crate) fn begin_epoch(&self, _generation: u64) {
        let mut state = self.state.lock().expect("frame mailbox mutex poisoned");
        self.epoch_fence.advance();
        if let Some(pending) = state.pending.take() {
            state.spare.push(pending);
        }
    }

    fn is_current(&self, identity: FrameIdentity) -> bool {
        self.epoch_fence.is_current(identity.epoch)
    }

    pub(crate) fn is_current_epoch(&self, epoch: MediaEpoch) -> bool {
        self.epoch_fence.is_current(epoch)
    }

    #[cfg(test)]
    fn commit_if_current<T>(&self, epoch: MediaEpoch, commit: impl FnOnce() -> T) -> Option<T> {
        self.epoch_fence.commit_if_current(epoch, commit)
    }
}

struct State {
    pending: Option<BufferGeneration>,
    spare: Vec<BufferGeneration>,
    newest: Option<FrameIdentity>,
}

pub(crate) struct BufferGeneration {
    identity: FrameIdentity,
    metadata: Option<CapturedFrameMetadata>,
    data: Box<[u8]>,
}

impl BufferGeneration {
    pub(crate) const fn generation(&self) -> u64 {
        self.identity.generation
    }

    pub(crate) const fn epoch(&self) -> MediaEpoch {
        self.identity.epoch
    }

    pub(crate) const fn data(&self) -> &[u8] {
        &self.data
    }

    pub(crate) const fn metadata(&self) -> Option<CapturedFrameMetadata> {
        self.metadata
    }
}

/// One owned native-dimension I420 generation taken from the latest-only handoff.
pub struct PublishedFrame(BufferGeneration);

impl PublishedFrame {
    /// Source generation represented by these bytes.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.0.generation()
    }

    /// Complete native-dimension I420 bytes.
    #[must_use]
    pub const fn data(&self) -> &[u8] {
        self.0.data()
    }

    pub(crate) const fn metadata(&self) -> Option<CapturedFrameMetadata> {
        self.0.metadata()
    }
}

fn new_buffer(buffer_bytes: usize) -> BufferGeneration {
    BufferGeneration {
        identity: FrameIdentity {
            epoch: MediaEpoch(0),
            generation: 0,
        },
        metadata: None,
        data: vec![0; buffer_bytes].into_boxed_slice(),
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FrameIdentity {
    epoch: MediaEpoch,
    generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct MediaEpoch(u64);

#[derive(Clone, Debug)]
pub(crate) struct EpochFence(Arc<AtomicU64>);

impl EpochFence {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicU64::new(0)))
    }

    fn current(&self) -> MediaEpoch {
        MediaEpoch(self.0.load(Ordering::Acquire))
    }

    fn advance(&self) -> MediaEpoch {
        let previous = self
            .0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |epoch| {
                Some(epoch.saturating_add(1))
            })
            .expect("semantic media epoch update is infallible");
        MediaEpoch(previous.saturating_add(1))
    }

    fn is_current(&self, epoch: MediaEpoch) -> bool {
        self.current() == epoch
    }

    #[cfg(test)]
    fn commit_if_current<T>(&self, epoch: MediaEpoch, commit: impl FnOnce() -> T) -> Option<T> {
        // This load is the encoder publication's commit point. An epoch that
        // advances afterward cannot relabel the already-committed frame, but
        // it also cannot wait on or be delayed by the downstream write.
        self.is_current(epoch).then(commit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_rate_same_epoch_input_preserves_claimed_work_and_only_latest_pending() {
        let mailbox = LatestBufferMailbox::new(1);
        let epoch = mailbox.current_epoch();
        mailbox.publish(epoch, 1, &[1]);
        let claimed = mailbox.take().expect("first generation is claimed");

        let mut skips = 0;
        for generation in 2..=100 {
            skips += mailbox.publish(
                epoch,
                generation,
                &[u8::try_from(generation).expect("bounded")],
            );
        }

        assert_eq!(skips, 98);
        assert!(mailbox.is_current_epoch(claimed.epoch()));
        let pending = mailbox.take().expect("one pending generation remains");
        assert_eq!(pending.generation(), 100);
        assert_eq!(pending.data(), &[100]);
    }

    #[test]
    fn advancing_a_shared_epoch_fences_every_handoff() {
        let epoch_fence = EpochFence::new();
        let raw = LatestBufferMailbox::with_epoch_fence(1, epoch_fence.clone());
        let processed = LatestBufferMailbox::with_epoch_fence(1, epoch_fence);
        let old_epoch = raw.current_epoch();
        raw.publish(old_epoch, 1, &[1]);
        let claimed = raw.take().expect("old raw generation is claimed");
        processed.publish(old_epoch, 1, &[1]);

        raw.begin_new_epoch();

        assert!(!raw.is_current_epoch(claimed.epoch()));
        assert!(processed.take().is_none(), "old processed output is fenced");
        raw.publish(old_epoch, 2, &[2]);
        assert!(raw.take().is_none(), "late old-epoch input is rejected");

        let current_epoch = raw.current_epoch();
        raw.publish(current_epoch, 1, &[3]);
        assert_eq!(
            raw.take()
                .expect("new epoch can restart at generation one")
                .data(),
            &[3]
        );
    }

    #[test]
    fn discarding_encoder_pending_work_preserves_the_shared_capture_epoch() {
        let epoch_fence = EpochFence::new();
        let raw = LatestBufferMailbox::with_epoch_fence(1, epoch_fence.clone());
        let processed = LatestBufferMailbox::with_epoch_fence(1, epoch_fence);
        let epoch = raw.current_epoch();
        raw.publish(epoch, 1, &[1]);
        let in_flight_capture = raw.take().expect("capture work is in flight");
        processed.publish(epoch, 1, &[1]);

        assert_eq!(processed.discard_pending(), 1);
        assert!(raw.is_current_epoch(in_flight_capture.epoch()));
        raw.publish(epoch, 2, &[2]);
        assert_eq!(
            raw.take().expect("capture epoch remains valid").data(),
            &[2]
        );
    }

    #[test]
    fn settings_boundary_advances_the_shared_epoch_without_reusing_a_recovery_epoch() {
        let mailbox = LatestBufferMailbox::new(1);
        mailbox.begin_epoch(3);

        assert_eq!(mailbox.current_epoch(), MediaEpoch(1));
        mailbox.begin_new_epoch();
        mailbox.begin_epoch(4);
        assert_eq!(mailbox.current_epoch(), MediaEpoch(3));
    }

    #[test]
    fn epoch_advance_rejects_uncommitted_old_output_without_waiting_on_committed_work() {
        let mailbox = LatestBufferMailbox::new(1);
        let old_epoch = mailbox.current_epoch();

        assert_eq!(
            mailbox.commit_if_current(old_epoch, || {
                mailbox.begin_new_epoch();
                42
            }),
            Some(42),
            "work committed before the boundary completes as the last old frame"
        );
        assert_ne!(mailbox.current_epoch(), old_epoch);
        assert_eq!(
            mailbox.commit_if_current(old_epoch, || 7),
            None,
            "old work not committed before the boundary is rejected"
        );
    }
}
