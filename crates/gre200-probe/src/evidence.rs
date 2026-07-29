use std::collections::{HashMap, HashSet, VecDeque};

use serde::Serialize;

use crate::contract::{
    BrowserPresentation, CorrelationResult, CorrelationStatus, SCHEMA_VERSION, Treatment,
};

pub(crate) const MAX_EVIDENCE_CLIENTS: usize = 16;
pub(crate) const SAMPLES_PER_CLIENT: usize = 512;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PresentationSample {
    pub(crate) runtime_epoch: String,
    pub(crate) stream_epoch: u64,
    pub(crate) connection_generation: u64,
    pub(crate) source_generation: Option<u64>,
    pub(crate) settings_generation: Option<u64>,
    pub(crate) treatment: Option<Treatment>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) visibility_state: String,
    pub(crate) presented_frames: u64,
    pub(crate) expected_display_unix_ms: f64,
    pub(crate) correlation_status: CorrelationStatus,
    pub(crate) exposure_end_to_visible_ms: Option<f64>,
    pub(crate) clock_uncertainty_ms: Option<f64>,
}

impl PresentationSample {
    fn new(
        presentation: &BrowserPresentation,
        result: &CorrelationResult,
        connection_generation: u64,
    ) -> Self {
        let frame = result.frame.as_ref();
        Self {
            runtime_epoch: presentation.runtime_epoch.clone(),
            stream_epoch: presentation.stream_epoch,
            connection_generation,
            source_generation: frame.map(|value| value.source_generation),
            settings_generation: frame.map(|value| value.settings_generation),
            treatment: frame.map(|value| value.treatment),
            width: presentation.width,
            height: presentation.height,
            visibility_state: presentation.visibility_state.clone(),
            presented_frames: presentation.presented_frames,
            expected_display_unix_ms: presentation.expected_display_unix_ms,
            correlation_status: result.status,
            exposure_end_to_visible_ms: result.exposure_end_to_visible_ms,
            clock_uncertainty_ms: presentation.clock_uncertainty_ms,
        }
    }

    fn key(&self) -> PartitionKey {
        PartitionKey {
            runtime_epoch: self.runtime_epoch.clone(),
            stream_epoch: self.stream_epoch,
            connection_generation: self.connection_generation,
            settings_generation: self.settings_generation,
            treatment: self.treatment,
            width: self.width,
            height: self.height,
            visibility_state: self.visibility_state.clone(),
        }
    }
}

#[derive(Clone, Debug)]
struct ClientWindow {
    samples: VecDeque<PresentationSample>,
    connection_generation: u64,
}

/// Bounded, RAM-only browser service-quality evidence.
#[derive(Clone, Debug)]
pub(crate) struct EvidenceStore {
    max_clients: usize,
    samples_per_client: usize,
    clients: HashMap<String, ClientWindow>,
    client_order: VecDeque<String>,
}

impl EvidenceStore {
    pub(crate) fn new() -> Self {
        Self::with_limits(MAX_EVIDENCE_CLIENTS, SAMPLES_PER_CLIENT)
    }

    fn with_limits(max_clients: usize, samples_per_client: usize) -> Self {
        assert!(max_clients > 0);
        assert!(samples_per_client > 0);
        Self {
            max_clients,
            samples_per_client,
            clients: HashMap::with_capacity(max_clients),
            client_order: VecDeque::with_capacity(max_clients),
        }
    }

    pub(crate) fn record(
        &mut self,
        presentation: &BrowserPresentation,
        result: &CorrelationResult,
    ) {
        let samples_per_client = self.samples_per_client;
        let window = self.touch_client(&presentation.client_id, 0);
        if window.samples.len() == samples_per_client {
            window.samples.pop_front();
        }
        let connection_generation = window.connection_generation.max(1);
        window.samples.push_back(PresentationSample::new(
            presentation,
            result,
            connection_generation,
        ));
    }

    pub(crate) fn record_connection(&mut self, client_id: &str) {
        let existed = self.clients.contains_key(client_id);
        let window = self.touch_client(client_id, 1);
        if existed {
            window.connection_generation = window.connection_generation.saturating_add(1);
        }
    }

    fn touch_client(
        &mut self,
        client_id: &str,
        initial_connection_generation: u64,
    ) -> &mut ClientWindow {
        if !self.clients.contains_key(client_id) {
            while self.clients.len() >= self.max_clients {
                if let Some(evicted) = self.client_order.pop_front() {
                    self.clients.remove(&evicted);
                }
            }
            self.clients.insert(
                client_id.to_owned(),
                ClientWindow {
                    samples: VecDeque::with_capacity(self.samples_per_client),
                    connection_generation: initial_connection_generation,
                },
            );
        } else if let Some(position) = self
            .client_order
            .iter()
            .position(|existing| existing == client_id)
        {
            self.client_order.remove(position);
        }
        self.client_order.push_back(client_id.to_owned());
        self.clients
            .get_mut(client_id)
            .expect("client window must exist after insertion")
    }

    pub(crate) fn snapshot(&self, runtime_epoch: &str) -> EvidenceSnapshot {
        let mut clients = self
            .clients
            .iter()
            .map(|(client_id, window)| client_evidence(client_id, window))
            .collect::<Vec<_>>();
        clients.sort_by(|left, right| left.client_id.cmp(&right.client_id));

        EvidenceSnapshot {
            schema_version: SCHEMA_VERSION,
            runtime_epoch: runtime_epoch.to_owned(),
            limits: EvidenceLimits {
                max_clients: self.max_clients,
                samples_per_client: self.samples_per_client,
            },
            combined: CombinedEvidence {
                client_count: clients.len(),
                sample_count: self
                    .clients
                    .values()
                    .map(|window| window.samples.len())
                    .sum(),
                reconnect_count: self
                    .clients
                    .values()
                    .map(|window| window.connection_generation.saturating_sub(1))
                    .sum(),
                partitions: aggregate(
                    self.clients
                        .values()
                        .flat_map(|window| window.samples.iter()),
                ),
            },
            clients,
        }
    }

    pub(crate) fn snapshot_for_client(
        &self,
        runtime_epoch: &str,
        client_id: &str,
    ) -> Option<EvidenceSnapshot> {
        let window = self.clients.get(client_id)?;
        let client = client_evidence(client_id, window);
        Some(EvidenceSnapshot {
            schema_version: SCHEMA_VERSION,
            runtime_epoch: runtime_epoch.to_owned(),
            limits: EvidenceLimits {
                max_clients: self.max_clients,
                samples_per_client: self.samples_per_client,
            },
            combined: CombinedEvidence {
                client_count: 1,
                sample_count: window.samples.len(),
                reconnect_count: window.connection_generation.saturating_sub(1),
                partitions: aggregate(window.samples.iter()),
            },
            clients: vec![client],
        })
    }
}

fn client_evidence(client_id: &str, window: &ClientWindow) -> ClientEvidence {
    ClientEvidence {
        client_id: client_id.to_owned(),
        sample_count: window.samples.len(),
        reconnect_count: window.connection_generation.saturating_sub(1),
        samples: window.samples.iter().cloned().collect(),
        partitions: aggregate(window.samples.iter()),
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct EvidenceSnapshot {
    pub(crate) schema_version: u8,
    pub(crate) runtime_epoch: String,
    pub(crate) limits: EvidenceLimits,
    pub(crate) combined: CombinedEvidence,
    pub(crate) clients: Vec<ClientEvidence>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct EvidenceLimits {
    pub(crate) max_clients: usize,
    pub(crate) samples_per_client: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CombinedEvidence {
    pub(crate) client_count: usize,
    pub(crate) sample_count: usize,
    pub(crate) reconnect_count: u64,
    pub(crate) partitions: Vec<PartitionEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ClientEvidence {
    pub(crate) client_id: String,
    pub(crate) sample_count: usize,
    pub(crate) reconnect_count: u64,
    pub(crate) samples: Vec<PresentationSample>,
    pub(crate) partitions: Vec<PartitionEvidence>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub(crate) struct PartitionKey {
    pub(crate) runtime_epoch: String,
    pub(crate) stream_epoch: u64,
    pub(crate) connection_generation: u64,
    pub(crate) settings_generation: Option<u64>,
    pub(crate) treatment: Option<Treatment>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) visibility_state: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PartitionEvidence {
    pub(crate) key: PartitionKey,
    pub(crate) sample_count: usize,
    pub(crate) unique_presented_frames: usize,
    pub(crate) unique_presented_cadence_hz: Option<f64>,
    pub(crate) exact_correlation_count: usize,
    pub(crate) unknown_correlation_count: usize,
    pub(crate) latency_ms: Distribution,
    pub(crate) clock_uncertainty_ms: Distribution,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct Distribution {
    pub(crate) sample_count: usize,
    pub(crate) min: Option<f64>,
    pub(crate) p50: Option<f64>,
    pub(crate) p95: Option<f64>,
    pub(crate) p99: Option<f64>,
    pub(crate) max: Option<f64>,
}

fn aggregate<'a>(samples: impl Iterator<Item = &'a PresentationSample>) -> Vec<PartitionEvidence> {
    let mut partitions: HashMap<PartitionKey, Vec<&PresentationSample>> = HashMap::new();
    for sample in samples {
        partitions.entry(sample.key()).or_default().push(sample);
    }
    let mut evidence = partitions
        .into_iter()
        .map(|(key, samples)| aggregate_partition(key, &samples))
        .collect::<Vec<_>>();
    evidence.sort_by(|left, right| {
        left.key
            .runtime_epoch
            .cmp(&right.key.runtime_epoch)
            .then(left.key.stream_epoch.cmp(&right.key.stream_epoch))
            .then(
                left.key
                    .connection_generation
                    .cmp(&right.key.connection_generation),
            )
            .then(
                left.key
                    .settings_generation
                    .cmp(&right.key.settings_generation),
            )
            .then(left.key.width.cmp(&right.key.width))
            .then(left.key.height.cmp(&right.key.height))
            .then(left.key.visibility_state.cmp(&right.key.visibility_state))
    });
    evidence
}

fn aggregate_partition(key: PartitionKey, samples: &[&PresentationSample]) -> PartitionEvidence {
    let exact_correlation_count = samples
        .iter()
        .filter(|sample| sample.correlation_status == CorrelationStatus::Correlated)
        .count();
    let mut unique_frames = HashSet::new();
    let mut first_unique_ms = None::<f64>;
    let mut last_unique_ms = None::<f64>;
    for sample in samples {
        if unique_frames.insert(sample.presented_frames) {
            first_unique_ms = Some(
                first_unique_ms.map_or(sample.expected_display_unix_ms, |value| {
                    value.min(sample.expected_display_unix_ms)
                }),
            );
            last_unique_ms = Some(
                last_unique_ms.map_or(sample.expected_display_unix_ms, |value| {
                    value.max(sample.expected_display_unix_ms)
                }),
            );
        }
    }
    let cadence = match (unique_frames.len(), first_unique_ms, last_unique_ms) {
        (count, Some(first), Some(last)) if count > 1 && last > first => {
            let intervals =
                u32::try_from(count - 1).expect("bounded client evidence count must fit into u32");
            Some(f64::from(intervals) * 1_000.0 / (last - first))
        }
        _ => None,
    };

    PartitionEvidence {
        key,
        sample_count: samples.len(),
        unique_presented_frames: unique_frames.len(),
        unique_presented_cadence_hz: cadence,
        exact_correlation_count,
        unknown_correlation_count: samples.len() - exact_correlation_count,
        latency_ms: distribution(
            samples
                .iter()
                .filter_map(|sample| sample.exposure_end_to_visible_ms),
        ),
        clock_uncertainty_ms: distribution(
            samples
                .iter()
                .filter_map(|sample| sample.clock_uncertainty_ms),
        ),
    }
}

fn distribution(values: impl Iterator<Item = f64>) -> Distribution {
    let mut values = values.collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    Distribution {
        sample_count: values.len(),
        min: values.first().copied(),
        p50: percentile(&values, 50),
        p95: percentile(&values, 95),
        p99: percentile(&values, 99),
        max: values.last().copied(),
    }
}

fn percentile(values: &[f64], percentile: usize) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let rank = (percentile * values.len()).div_ceil(100);
    values.get(rank.saturating_sub(1)).copied()
}

#[cfg(test)]
mod tests {
    use crate::contract::{
        BrowserPresentation, CorrelationResult, CorrelationStatus, FrameMapping, SCHEMA_VERSION,
        Treatment,
    };

    use super::EvidenceStore;

    fn presentation(
        client_id: &str,
        presented_frames: u64,
        visible_ms: f64,
    ) -> BrowserPresentation {
        BrowserPresentation {
            schema_version: SCHEMA_VERSION,
            client_id: client_id.into(),
            runtime_epoch: "runtime1".into(),
            stream_epoch: 1,
            rtp_timestamp: Some(
                u32::try_from(presented_frames).expect("test frame count must fit into u32"),
            ),
            expected_display_unix_ms: visible_ms,
            clock_uncertainty_ms: Some(2.0),
            presented_frames,
            width: 1920,
            height: 1080,
            visibility_state: "visible".into(),
        }
    }

    fn correlated(generation: u64, latency_ms: f64) -> CorrelationResult {
        CorrelationResult {
            status: CorrelationStatus::Correlated,
            observed_rtp_timestamp: Some(
                u32::try_from(generation).expect("test generation must fit into u32"),
            ),
            frame: Some(FrameMapping {
                schema_version: SCHEMA_VERSION,
                runtime_epoch: "runtime1".into(),
                stream_epoch: 1,
                rtp_timestamp: u32::try_from(generation)
                    .expect("test generation must fit into u32"),
                source_generation: generation,
                settings_generation: 7,
                treatment: Treatment::Mono,
                exposure_completed_unix_ns: 1,
                submitted_unix_ns: 2,
            }),
            exposure_end_to_visible_ms: Some(latency_ms),
            clock_uncertainty_ms: Some(2.0),
        }
    }

    #[test]
    fn evidence_is_bounded_per_client_and_across_clients() {
        let mut store = EvidenceStore::with_limits(2, 2);
        for frame in 1..=3 {
            store.record(
                &presentation("client1", frame, test_f64(frame) * 10.0),
                &correlated(
                    frame,
                    f64::from(u32::try_from(frame).expect("test frame must fit into u32")),
                ),
            );
        }
        store.record(&presentation("client2", 1, 10.0), &correlated(1, 1.0));
        store.record(&presentation("client3", 1, 10.0), &correlated(1, 1.0));

        let evidence = store.snapshot("runtime1");
        assert_eq!(evidence.limits.max_clients, 2);
        assert_eq!(evidence.limits.samples_per_client, 2);
        assert_eq!(evidence.clients.len(), 2);
        assert!(
            evidence
                .clients
                .iter()
                .all(|client| client.client_id != "client1")
        );
        assert!(
            evidence
                .clients
                .iter()
                .all(|client| client.sample_count <= 2)
        );
    }

    #[test]
    fn aggregates_use_worked_percentiles_and_unique_presented_cadence() {
        let mut store = EvidenceStore::with_limits(4, 8);
        for (frame, latency) in [(1, 10.0), (2, 20.0), (3, 30.0), (4, 40.0)] {
            store.record(
                &presentation("client1", frame, test_f64(frame) * 100.0),
                &correlated(frame, latency),
            );
        }
        store.record(&presentation("client1", 4, 450.0), &correlated(4, 99.0));

        let partition = &store.snapshot("runtime1").clients[0].partitions[0];
        assert_eq!(partition.sample_count, 5);
        assert_eq!(partition.unique_presented_frames, 4);
        assert_eq!(partition.exact_correlation_count, 5);
        assert_eq!(partition.latency_ms.sample_count, 5);
        assert_eq!(partition.latency_ms.min, Some(10.0));
        assert_eq!(partition.latency_ms.p50, Some(30.0));
        assert_eq!(partition.latency_ms.p95, Some(99.0));
        assert_eq!(partition.latency_ms.p99, Some(99.0));
        assert_eq!(partition.latency_ms.max, Some(99.0));
        assert!((partition.unique_presented_cadence_hz.unwrap() - 10.0).abs() < 0.001);
        assert_eq!(
            store.snapshot("runtime1").clients[0]
                .samples
                .iter()
                .filter_map(|sample| sample.source_generation)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 4],
        );
    }

    #[test]
    fn visibility_generation_and_treatment_are_separate_partitions() {
        let mut store = EvidenceStore::with_limits(4, 8);
        let first = presentation("client1", 1, 100.0);
        store.record(&first, &correlated(1, 10.0));

        let mut hidden = presentation("client1", 2, 200.0);
        hidden.visibility_state = "hidden".into();
        store.record(&hidden, &correlated(2, 20.0));

        let mut colour = correlated(3, 30.0);
        let frame = colour.frame.as_mut().unwrap();
        frame.settings_generation = 8;
        frame.treatment = Treatment::Colour;
        store.record(&presentation("client1", 3, 300.0), &colour);

        let evidence = store.snapshot("runtime1");
        assert_eq!(evidence.clients[0].partitions.len(), 3);
        assert_eq!(evidence.combined.partitions.len(), 3);
    }

    #[test]
    fn reconnects_are_counted_and_partitioned() {
        let mut store = EvidenceStore::with_limits(4, 8);
        store.record_connection("client1");
        store.record(&presentation("client1", 1, 100.0), &correlated(1, 10.0));
        store.record_connection("client1");
        store.record(&presentation("client1", 2, 200.0), &correlated(2, 20.0));

        let evidence = store.snapshot("runtime1");
        assert_eq!(evidence.clients[0].reconnect_count, 1);
        assert_eq!(evidence.clients[0].partitions.len(), 2);
        assert_eq!(evidence.combined.reconnect_count, 1);
    }

    #[test]
    fn runtime_and_stream_epochs_are_separate_partitions() {
        let mut store = EvidenceStore::with_limits(4, 8);
        store.record(&presentation("client1", 1, 100.0), &correlated(1, 10.0));
        let mut restarted = presentation("client1", 2, 200.0);
        restarted.runtime_epoch = "runtime2".into();
        restarted.stream_epoch = 2;
        store.record(&restarted, &correlated(2, 20.0));

        let evidence = store.snapshot("runtime2");
        assert_eq!(evidence.clients[0].partitions.len(), 2);
        assert_eq!(evidence.combined.partitions.len(), 2);
    }

    #[test]
    fn client_snapshot_does_not_aggregate_other_viewers() {
        let mut store = EvidenceStore::with_limits(4, 8);
        store.record(&presentation("client1", 1, 100.0), &correlated(1, 10.0));
        store.record(&presentation("client2", 1, 100.0), &correlated(1, 20.0));

        let evidence = store
            .snapshot_for_client("runtime1", "client1")
            .expect("recorded client must have evidence");
        assert_eq!(evidence.clients.len(), 1);
        assert_eq!(evidence.clients[0].client_id, "client1");
        assert_eq!(evidence.combined.client_count, 1);
        assert_eq!(evidence.combined.sample_count, 1);
        assert_eq!(evidence.combined.partitions[0].latency_ms.p50, Some(10.0));
    }

    #[test]
    fn unknown_correlation_is_counted_without_inventing_frame_identity() {
        let mut store = EvidenceStore::with_limits(4, 8);
        let unknown = CorrelationResult {
            status: CorrelationStatus::UnknownAmbiguous,
            observed_rtp_timestamp: Some(1),
            frame: None,
            exposure_end_to_visible_ms: None,
            clock_uncertainty_ms: None,
        };
        store.record(&presentation("client1", 1, 100.0), &unknown);

        let partition = &store.snapshot("runtime1").clients[0].partitions[0];
        assert_eq!(partition.exact_correlation_count, 0);
        assert_eq!(partition.unknown_correlation_count, 1);
        assert_eq!(
            store.snapshot("runtime1").clients[0].samples[0].source_generation,
            None
        );
        assert_eq!(partition.key.settings_generation, None);
        assert_eq!(partition.key.treatment, None);
        assert_eq!(partition.latency_ms.sample_count, 0);
    }

    fn test_f64(value: u64) -> f64 {
        f64::from(u32::try_from(value).expect("test value must fit into u32"))
    }
}
