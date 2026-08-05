use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use serde::Serialize;
use thiserror::Error;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::Treatment;

const CORRELATION_CAPACITY: usize = 128;
const RTP_INPUT_STEP: u32 = 4_500;

/// Capture facts that remain attached through processing and publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CapturedFrameMetadata {
    pub(crate) settings_generation: u64,
    pub(crate) treatment: Treatment,
    pub(crate) exposure_completed_at_unix_us: u64,
}

/// Immutable identity and timing recorded for one `FFmpeg` input submission.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameSubmission {
    runtime_epoch: Uuid,
    stream_epoch: u64,
    source_generation: u64,
    settings_generation: u64,
    treatment: Treatment,
    width: u16,
    height: u16,
    exposure_completed_at_unix_us: u64,
    submitted_at_unix_us: u64,
    repeat: bool,
}

impl FrameSubmission {
    /// Creates the complete evidence attached to one encoder input step.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        runtime_epoch: Uuid,
        stream_epoch: u64,
        source_generation: u64,
        settings_generation: u64,
        treatment: Treatment,
        width: u16,
        height: u16,
        exposure_completed_at_unix_us: u64,
        submitted_at_unix_us: u64,
        repeat: bool,
    ) -> Self {
        Self {
            runtime_epoch,
            stream_epoch,
            source_generation,
            settings_generation,
            treatment,
            width,
            height,
            exposure_completed_at_unix_us,
            submitted_at_unix_us,
            repeat,
        }
    }

    /// Marks another input step as a repeat of this completed generation.
    #[must_use]
    pub const fn as_repeat(mut self) -> Self {
        self.repeat = true;
        self
    }
}

/// One exact browser-matchable RTP mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrelationMapping {
    rtp_timestamp: u32,
    input_index: u64,
    #[serde(flatten)]
    submission: FrameSubmission,
}

impl CorrelationMapping {
    pub(crate) const fn runtime_epoch(self) -> Uuid {
        self.submission.runtime_epoch
    }

    pub(crate) const fn stream_epoch(self) -> u64 {
        self.submission.stream_epoch
    }

    /// Observed 90 kHz RTP timestamp.
    #[must_use]
    pub const fn rtp_timestamp(self) -> u32 {
        self.rtp_timestamp
    }

    /// Original source generation; repeats retain this identity.
    #[must_use]
    pub const fn source_generation(self) -> u64 {
        self.submission.source_generation
    }

    pub(crate) const fn settings_generation(self) -> u64 {
        self.submission.settings_generation
    }

    pub(crate) const fn treatment(self) -> Treatment {
        self.submission.treatment
    }

    pub(crate) const fn dimensions(self) -> (u16, u16) {
        (self.submission.width, self.submission.height)
    }

    pub(crate) const fn exposure_completed_at_unix_us(self) -> u64 {
        self.submission.exposure_completed_at_unix_us
    }

    /// Whether this input was a bounded repeat of the completed generation.
    #[must_use]
    pub const fn is_repeat(self) -> bool {
        self.submission.repeat
    }
}

/// Reason exact correlation could not be established.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum CorrelationError {
    /// The referenced input was already evicted from the bounded queue.
    #[error("encoder input mapping was evicted")]
    Evicted,
    /// No exact RTP/input anchor has been observed.
    #[error("RTP timeline is not anchored")]
    Unanchored,
    /// The RTP delta is not a whole encoder input step.
    #[error("RTP timestamp gap is fractional")]
    FractionalGap,
    /// The wrapping RTP delta is exactly half the timestamp space.
    #[error("RTP timestamp gap is ambiguous")]
    AmbiguousGap,
    /// The observation points behind the established timeline.
    #[error("RTP timestamp reset or stale observation")]
    Reset,
    /// One timestamp was associated with conflicting input evidence.
    #[error("RTP timestamp has conflicting mappings")]
    Conflict,
}

#[derive(Clone, Copy, Debug)]
struct PendingSubmission {
    input_index: u64,
    submission: FrameSubmission,
}

/// Bounded fail-closed mapping from `FFmpeg` input steps to observed RTP timestamps.
#[derive(Debug)]
pub struct CorrelationTracker {
    capacity: usize,
    rtp_step: u32,
    next_input_index: u64,
    pending: VecDeque<PendingSubmission>,
    mappings: VecDeque<CorrelationMapping>,
    poisoned: VecDeque<u32>,
    anchor: Option<(u64, u32)>,
    last_observed_index: Option<u64>,
    encoder_skips: u64,
}

#[derive(Debug)]
struct CorrelationRuntime {
    stream_epoch: u64,
    tracker: CorrelationTracker,
}

/// Runtime-local exact-correlation feed shared by the encoder and browsers.
#[derive(Clone, Debug)]
pub(crate) struct CorrelationState {
    runtime_epoch: Uuid,
    inner: Arc<Mutex<CorrelationRuntime>>,
    updates: broadcast::Sender<CorrelationMapping>,
}

impl CorrelationState {
    pub(crate) fn new(runtime_epoch: Uuid) -> Self {
        let (updates, _) = broadcast::channel(CORRELATION_CAPACITY);
        Self {
            runtime_epoch,
            inner: Arc::new(Mutex::new(CorrelationRuntime {
                stream_epoch: 0,
                tracker: CorrelationTracker::new(CORRELATION_CAPACITY, RTP_INPUT_STEP),
            })),
            updates,
        }
    }

    pub(crate) fn begin_stream(&self) -> u64 {
        let mut inner = self.inner.lock().expect("correlation mutex poisoned");
        inner.stream_epoch = inner
            .stream_epoch
            .checked_add(1)
            .expect("stream epoch exhausted");
        inner.tracker = CorrelationTracker::new(CORRELATION_CAPACITY, RTP_INPUT_STEP);
        inner.stream_epoch
    }

    pub(crate) fn submit(
        &self,
        stream_epoch: u64,
        metadata: CapturedFrameMetadata,
        source_generation: u64,
        submitted_at_unix_us: u64,
        repeat: bool,
    ) -> Option<u64> {
        let mut inner = self.inner.lock().expect("correlation mutex poisoned");
        if inner.stream_epoch != stream_epoch {
            return None;
        }
        let submission = FrameSubmission::new(
            self.runtime_epoch,
            stream_epoch,
            source_generation,
            metadata.settings_generation,
            metadata.treatment,
            1920,
            1080,
            metadata.exposure_completed_at_unix_us,
            submitted_at_unix_us,
            repeat,
        );
        Some(inner.tracker.submit(submission))
    }

    pub(crate) fn skip_submission(&self, stream_epoch: u64) -> Option<u64> {
        let mut inner = self.inner.lock().expect("correlation mutex poisoned");
        (inner.stream_epoch == stream_epoch).then(|| inner.tracker.skip_submission())
    }

    pub(crate) fn observe(
        &self,
        stream_epoch: u64,
        input_index: u64,
        rtp_timestamp: u32,
    ) -> Result<CorrelationMapping, CorrelationError> {
        let mut inner = self.inner.lock().expect("correlation mutex poisoned");
        if inner.stream_epoch != stream_epoch {
            return Err(CorrelationError::Reset);
        }
        let mapping = inner.tracker.anchor(input_index, rtp_timestamp)?;
        let _ = self.updates.send(mapping);
        Ok(mapping)
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<CorrelationMapping> {
        self.updates.subscribe()
    }

    pub(crate) fn lookup(
        &self,
        stream_epoch: u64,
        rtp_timestamp: u32,
    ) -> Option<CorrelationMapping> {
        let inner = self.inner.lock().expect("correlation mutex poisoned");
        (inner.stream_epoch == stream_epoch)
            .then(|| inner.tracker.lookup(rtp_timestamp).copied())
            .flatten()
    }
}

impl CorrelationTracker {
    /// Creates a tracker with a fixed RAM bound and 90 kHz input step.
    ///
    /// # Panics
    ///
    /// Panics when capacity or the input step is zero.
    #[must_use]
    pub fn new(capacity: usize, rtp_step: u32) -> Self {
        assert!(capacity > 0, "correlation capacity must be non-zero");
        assert!(rtp_step > 0, "RTP input step must be non-zero");
        Self {
            capacity,
            rtp_step,
            next_input_index: 0,
            pending: VecDeque::with_capacity(capacity),
            mappings: VecDeque::with_capacity(capacity),
            poisoned: VecDeque::with_capacity(capacity),
            anchor: None,
            last_observed_index: None,
            encoder_skips: 0,
        }
    }

    /// Records one complete submission and returns its input-timeline index.
    ///
    /// # Panics
    ///
    /// Panics if the process exhausts the `u64` input timeline.
    pub fn submit(&mut self, submission: FrameSubmission) -> u64 {
        let input_index = self.next_input_index;
        self.next_input_index = self
            .next_input_index
            .checked_add(1)
            .expect("encoder input timeline exhausted");
        if self.pending.len() == self.capacity {
            self.pending.pop_front();
        }
        self.pending.push_back(PendingSubmission {
            input_index,
            submission,
        });
        input_index
    }

    fn skip_submission(&mut self) -> u64 {
        let input_index = self.next_input_index;
        self.next_input_index = self
            .next_input_index
            .checked_add(1)
            .expect("encoder input timeline exhausted");
        input_index
    }

    /// Establishes an exact output/input point from direct encoder evidence.
    ///
    /// # Errors
    ///
    /// Returns a fail-closed reason when the input was evicted, the timestamp
    /// conflicts, or the observation moves behind the established timeline.
    pub fn anchor(
        &mut self,
        input_index: u64,
        rtp_timestamp: u32,
    ) -> Result<CorrelationMapping, CorrelationError> {
        if let Some(existing) = self.lookup(rtp_timestamp).copied() {
            if existing.input_index != input_index {
                self.poison(rtp_timestamp);
                return Err(CorrelationError::Conflict);
            }
            return Ok(existing);
        }
        if self.poisoned.contains(&rtp_timestamp) {
            return Err(CorrelationError::Conflict);
        }
        let mapping = self.map_index(input_index, rtp_timestamp)?;
        self.anchor = Some((input_index, rtp_timestamp));
        Ok(mapping)
    }

    /// Maps an observed RTP timestamp by the anchored input timeline.
    ///
    /// # Errors
    ///
    /// Returns a fail-closed reason when no anchor exists or the wrapping gap
    /// is fractional, ambiguous, reset, conflicting, or no longer retained.
    pub fn observe(&mut self, rtp_timestamp: u32) -> Result<CorrelationMapping, CorrelationError> {
        if self.poisoned.contains(&rtp_timestamp) {
            return Err(CorrelationError::Conflict);
        }
        if let Some(existing) = self.lookup(rtp_timestamp).copied() {
            return Ok(existing);
        }
        let (anchor_index, anchor_timestamp) = self.anchor.ok_or(CorrelationError::Unanchored)?;
        let delta = rtp_timestamp.wrapping_sub(anchor_timestamp);
        if delta == 0x8000_0000 {
            return Err(CorrelationError::AmbiguousGap);
        }
        if delta > 0x8000_0000 {
            return Err(CorrelationError::Reset);
        }
        if delta % self.rtp_step != 0 {
            return Err(CorrelationError::FractionalGap);
        }
        let input_index = anchor_index
            .checked_add(u64::from(delta / self.rtp_step))
            .ok_or(CorrelationError::Reset)?;
        let mapping = self.map_index(input_index, rtp_timestamp)?;
        self.anchor = Some((input_index, rtp_timestamp));
        Ok(mapping)
    }

    /// Looks up only retained, non-conflicting exact evidence.
    #[must_use]
    pub fn lookup(&self, rtp_timestamp: u32) -> Option<&CorrelationMapping> {
        (!self.poisoned.contains(&rtp_timestamp))
            .then(|| {
                self.mappings
                    .iter()
                    .find(|mapping| mapping.rtp_timestamp == rtp_timestamp)
            })
            .flatten()
    }

    /// Number of whole input steps proven absent from encoder output.
    #[must_use]
    pub const fn encoder_skips(&self) -> u64 {
        self.encoder_skips
    }

    fn map_index(
        &mut self,
        input_index: u64,
        rtp_timestamp: u32,
    ) -> Result<CorrelationMapping, CorrelationError> {
        let position = self
            .pending
            .iter()
            .position(|pending| pending.input_index == input_index)
            .ok_or(CorrelationError::Evicted)?;
        if let Some(previous) = self.last_observed_index {
            if input_index <= previous {
                return Err(CorrelationError::Reset);
            }
            self.encoder_skips = self
                .encoder_skips
                .saturating_add(input_index.saturating_sub(previous).saturating_sub(1));
        }
        let pending = self.pending[position];
        self.pending
            .retain(|candidate| candidate.input_index > input_index);
        let mapping = CorrelationMapping {
            rtp_timestamp,
            input_index,
            submission: pending.submission,
        };
        if self.mappings.len() == self.capacity {
            self.mappings.pop_front();
        }
        self.mappings.push_back(mapping);
        self.last_observed_index = Some(input_index);
        Ok(mapping)
    }

    fn poison(&mut self, rtp_timestamp: u32) {
        self.mappings
            .retain(|mapping| mapping.rtp_timestamp != rtp_timestamp);
        if self.poisoned.len() == self.capacity {
            self.poisoned.pop_front();
        }
        self.poisoned.push_back(rtp_timestamp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn submission(source_generation: u64) -> FrameSubmission {
        FrameSubmission::new(
            Uuid::nil(),
            1,
            source_generation,
            0,
            Treatment::Monochrome,
            1920,
            1080,
            1,
            2,
            false,
        )
    }

    #[test]
    fn direct_pts_map_an_encoder_input_skip_without_inventing_a_frame() {
        let mut tracker = CorrelationTracker::new(4, 4_500);
        tracker.submit(submission(1));
        tracker.submit(submission(2));
        tracker.submit(submission(3));

        assert_eq!(tracker.anchor(0, 90_000).unwrap().source_generation(), 1);
        assert_eq!(tracker.anchor(2, 94_500).unwrap().source_generation(), 3);
        assert_eq!(tracker.encoder_skips(), 1);
        assert!(tracker.lookup(94_500).is_some());
    }

    #[test]
    fn replacing_a_stream_invalidates_every_prior_epoch_mapping() {
        let state = CorrelationState::new(Uuid::from_u128(1));
        let first_stream = state.begin_stream();
        state.submit(
            first_stream,
            CapturedFrameMetadata {
                settings_generation: 0,
                treatment: Treatment::Monochrome,
                exposure_completed_at_unix_us: 1,
            },
            7,
            2,
            false,
        );
        state
            .observe(first_stream, 0, 90_000)
            .expect("first stream mapping");

        let replacement = state.begin_stream();

        assert!(replacement > first_stream);
        assert!(state.lookup(first_stream, 90_000).is_none());
        assert_eq!(
            state.observe(first_stream, 0, 90_000),
            Err(CorrelationError::Reset)
        );
    }

    #[test]
    fn an_untrusted_input_advances_the_timeline_without_creating_a_mapping() {
        let state = CorrelationState::new(Uuid::from_u128(1));
        let stream = state.begin_stream();
        assert_eq!(state.skip_submission(stream), Some(0));
        assert_eq!(
            state.submit(
                stream,
                CapturedFrameMetadata {
                    settings_generation: 1,
                    treatment: Treatment::Monochrome,
                    exposure_completed_at_unix_us: 1,
                },
                7,
                2,
                false,
            ),
            Some(1)
        );

        assert_eq!(
            state.observe(stream, 0, 90_000),
            Err(CorrelationError::Evicted)
        );
        assert_eq!(
            state
                .observe(stream, 1, 94_500)
                .expect("trusted input after transition")
                .source_generation(),
            7
        );
    }
}
