use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::broadcast;

use crate::contract::{
    BrowserPresentation, CaptureProgress, CorrelationResult, CorrelationStatus, FrameMapping,
    SCHEMA_VERSION, SubmittedFrame,
};

const MAPPING_CAPACITY: usize = 4096;
pub(crate) const MAX_PENDING_SUBMISSIONS: usize = 8;

#[derive(Debug)]
struct Inner {
    stream_epoch: u64,
    pending: VecDeque<SubmittedFrame>,
    mappings: HashMap<u32, FrameMapping>,
    order: VecDeque<u32>,
    poisoned: HashSet<u32>,
    capture: Option<CaptureProgress>,
}

#[derive(Clone, Debug)]
pub struct ProbeState {
    runtime_epoch: Arc<str>,
    inner: Arc<Mutex<Inner>>,
    mapping_tx: broadcast::Sender<FrameMapping>,
    capture_tx: broadcast::Sender<CaptureProgress>,
}

impl ProbeState {
    pub fn new(runtime_epoch: String, stream_epoch: u64) -> Self {
        let (mapping_tx, _) = broadcast::channel(64);
        let (capture_tx, _) = broadcast::channel(16);
        Self {
            runtime_epoch: runtime_epoch.into(),
            inner: Arc::new(Mutex::new(Inner {
                stream_epoch,
                pending: VecDeque::with_capacity(4),
                mappings: HashMap::with_capacity(MAPPING_CAPACITY),
                order: VecDeque::with_capacity(MAPPING_CAPACITY),
                poisoned: HashSet::new(),
                capture: None,
            })),
            mapping_tx,
            capture_tx,
        }
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn runtime_epoch(&self) -> &str {
        &self.runtime_epoch
    }

    pub fn stream_epoch(&self) -> u64 {
        self.inner().stream_epoch
    }

    pub fn begin_stream_epoch(&self) -> u64 {
        let mut inner = self.inner();
        inner.stream_epoch = inner.stream_epoch.wrapping_add(1);
        inner.pending.clear();
        inner.mappings.clear();
        inner.order.clear();
        inner.poisoned.clear();
        inner.stream_epoch
    }

    pub fn record_submission(&self, frame: SubmittedFrame) -> bool {
        let mut inner = self.inner();
        if inner.pending.len() >= MAX_PENDING_SUBMISSIONS {
            inner.pending.clear();
            return false;
        }
        inner.pending.push_back(frame);
        true
    }

    pub fn abandon_submissions(&self) {
        self.inner().pending.clear();
    }

    pub fn record_rtp_timestamp(
        &self,
        rtp_timestamp: u32,
        skipped_submissions: usize,
    ) -> Option<FrameMapping> {
        let mapping = {
            let mut inner = self.inner();
            if inner.mappings.contains_key(&rtp_timestamp)
                || inner.poisoned.contains(&rtp_timestamp)
            {
                return None;
            }
            for _ in 0..skipped_submissions {
                inner.pending.pop_front()?;
            }
            let submitted = inner.pending.pop_front()?;
            let mapping = FrameMapping {
                schema_version: SCHEMA_VERSION,
                runtime_epoch: self.runtime_epoch.to_string(),
                stream_epoch: inner.stream_epoch,
                rtp_timestamp,
                source_generation: submitted.source_generation,
                settings_generation: submitted.settings_generation,
                treatment: submitted.treatment,
                exposure_completed_unix_ns: submitted.exposure_completed_unix_ns,
                submitted_unix_ns: submitted.submitted_unix_ns,
            };
            if let Some(existing) = inner.mappings.insert(rtp_timestamp, mapping.clone())
                && existing != mapping
            {
                inner.mappings.remove(&rtp_timestamp);
                inner.poisoned.insert(rtp_timestamp);
                return None;
            }
            inner.order.push_back(rtp_timestamp);
            while inner.order.len() > MAPPING_CAPACITY {
                if let Some(expired) = inner.order.pop_front() {
                    inner.mappings.remove(&expired);
                    inner.poisoned.remove(&expired);
                }
            }
            mapping
        };
        let _ = self.mapping_tx.send(mapping.clone());
        Some(mapping)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<FrameMapping> {
        self.mapping_tx.subscribe()
    }

    pub fn record_capture_started(
        &self,
        settings_generation: u64,
        source_generation: u64,
        exposure_us: i64,
        capture_started_unix_ns: u128,
    ) {
        let capture = CaptureProgress {
            schema_version: SCHEMA_VERSION,
            runtime_epoch: self.runtime_epoch.to_string(),
            settings_generation,
            source_generation,
            exposure_us,
            capture_started_unix_ns,
        };
        self.inner().capture = Some(capture.clone());
        let _ = self.capture_tx.send(capture);
    }

    pub fn capture(&self) -> Option<CaptureProgress> {
        self.inner().capture.clone()
    }

    pub fn subscribe_capture(&self) -> broadcast::Receiver<CaptureProgress> {
        self.capture_tx.subscribe()
    }

    pub fn snapshot(&self) -> Vec<FrameMapping> {
        let inner = self.inner();
        inner
            .order
            .iter()
            .filter_map(|timestamp| inner.mappings.get(timestamp).cloned())
            .collect()
    }

    pub fn resolve(&self, presentation: &BrowserPresentation) -> CorrelationResult {
        let observed = presentation.rtp_timestamp;
        if presentation.runtime_epoch != self.runtime_epoch.as_ref()
            || presentation.stream_epoch != self.stream_epoch()
        {
            return unknown(CorrelationStatus::UnknownEpoch, observed);
        }
        let Some(timestamp) = observed else {
            return unknown(CorrelationStatus::UnknownNoRtpTimestamp, None);
        };
        let inner = self.inner();
        if inner.poisoned.contains(&timestamp) {
            return unknown(CorrelationStatus::UnknownAmbiguous, observed);
        }
        let Some(frame) = inner.mappings.get(&timestamp).cloned() else {
            return unknown(CorrelationStatus::UnknownMissing, observed);
        };
        let uncertainty = presentation.clock_uncertainty_ms;
        let latency = uncertainty.map(|_| {
            presentation.expected_display_unix_ms
                - unix_ns_to_millis(frame.exposure_completed_unix_ns)
        });
        CorrelationResult {
            status: CorrelationStatus::Correlated,
            observed_rtp_timestamp: observed,
            frame: Some(frame),
            exposure_end_to_visible_ms: latency,
            clock_uncertainty_ms: uncertainty,
        }
    }
}

fn unix_ns_to_millis(value: u128) -> f64 {
    let nanos = u64::try_from(value).unwrap_or(u64::MAX);
    std::time::Duration::from_nanos(nanos).as_secs_f64() * 1_000.0
}

fn unknown(status: CorrelationStatus, observed: Option<u32>) -> CorrelationResult {
    CorrelationResult {
        status,
        observed_rtp_timestamp: observed,
        frame: None,
        exposure_end_to_visible_ms: None,
        clock_uncertainty_ms: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Treatment;

    #[test]
    fn maps_oldest_submitted_frame_to_new_rtp_timestamp() {
        let state = ProbeState::new("epoch".into(), 1);
        assert!(state.record_submission(SubmittedFrame {
            source_generation: 7,
            settings_generation: 2,
            treatment: Treatment::Mono,
            exposure_completed_unix_ns: 10,
            submitted_unix_ns: 11,
        }));

        let mapping = state.record_rtp_timestamp(123, 0).expect("mapping");

        assert_eq!(mapping.source_generation, 7);
        assert_eq!(state.snapshot(), vec![mapping]);
    }

    #[test]
    fn missing_browser_timestamp_is_unknown() {
        let state = ProbeState::new("epoch".into(), 1);
        let result = state.resolve(&BrowserPresentation {
            schema_version: 1,
            client_id: "client".into(),
            runtime_epoch: "epoch".into(),
            stream_epoch: 1,
            rtp_timestamp: None,
            expected_display_unix_ms: 1.0,
            clock_uncertainty_ms: None,
            presented_frames: 1,
            width: 1920,
            height: 1080,
            visibility_state: "visible".into(),
        });

        assert_eq!(result.status, CorrelationStatus::UnknownNoRtpTimestamp);
    }

    #[test]
    fn new_stream_epoch_invalidates_old_mappings() {
        let state = ProbeState::new("epoch".into(), 4);
        assert!(state.record_submission(SubmittedFrame {
            source_generation: 7,
            settings_generation: 2,
            treatment: Treatment::Mono,
            exposure_completed_unix_ns: 10,
            submitted_unix_ns: 11,
        }));
        assert!(state.record_rtp_timestamp(123, 0).is_some());

        assert_eq!(state.begin_stream_epoch(), 5);
        assert!(state.snapshot().is_empty());
    }

    #[test]
    fn skipped_encoder_input_advances_to_matching_submission() {
        let state = ProbeState::new("epoch".into(), 1);
        for source_generation in [7, 7, 8] {
            assert!(state.record_submission(SubmittedFrame {
                source_generation,
                settings_generation: 2,
                treatment: Treatment::Mono,
                exposure_completed_unix_ns: 10,
                submitted_unix_ns: 11,
            }));
        }

        assert_eq!(
            state
                .record_rtp_timestamp(123, 0)
                .expect("first mapping")
                .source_generation,
            7
        );
        assert_eq!(
            state
                .record_rtp_timestamp(125, 1)
                .expect("mapping after skipped heartbeat")
                .source_generation,
            8
        );
    }
}
