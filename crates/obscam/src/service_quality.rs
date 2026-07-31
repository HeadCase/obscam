use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    CorrelationMapping, Treatment, correlation::CorrelationState, runtime::SCHEMA_VERSION,
};

pub(crate) const MAX_CLIENTS: usize = 16;
pub(crate) const MAX_SAMPLES_PER_CLIENT: usize = 512;
const MAX_CLOCK_UNCERTAINTY_US: u64 = 5_000_000;
const MAX_PRESENTATION_CLOCK_SKEW_US: u64 = 60_000_000;
const MAX_REPORT_BATCH: usize = 32;

#[derive(Clone, Debug)]
pub(crate) struct ServiceQualityState {
    runtime_epoch: Uuid,
    correlation: CorrelationState,
    inner: Arc<Mutex<EvidenceStore>>,
}

#[derive(Debug, Default)]
struct EvidenceStore {
    activity: u64,
    clients: Vec<ClientEvidence>,
}

#[derive(Clone, Debug)]
struct ClientEvidence {
    client_id: Uuid,
    last_activity: u64,
    connection_generation: u64,
    reconnects: u64,
    presentation_skips: u64,
    last_presented_frames: Option<(u64, u64)>,
    samples: VecDeque<PresentationSample>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionRequest {
    schema_version: u8,
    client_id: Uuid,
    runtime_epoch: Uuid,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionResponse {
    schema_version: u8,
    client_id: Uuid,
    connection_generation: u64,
    reconnects: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PresentationBatch {
    schema_version: u8,
    client_id: Uuid,
    runtime_epoch: Uuid,
    connection_generation: u64,
    samples: Vec<PresentationObservation>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PresentationObservation {
    stream_epoch: Option<u64>,
    rtp_timestamp: Option<u32>,
    presented_frames: u64,
    presented_at_unix_us: u64,
    clock_uncertainty_us: u64,
    visibility: Visibility,
    correlation: ReportedCorrelation,
}

#[derive(Clone, Copy, Debug)]
struct PresentationReport {
    #[cfg(test)]
    client_id: Uuid,
    runtime_epoch: Uuid,
    connection_generation: u64,
    observation: PresentationObservation,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReportedCorrelation {
    Exact,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Visibility {
    Visible,
    Hidden,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CorrelationStatus {
    Exact,
    Unknown,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PresentationSample {
    runtime_epoch: Uuid,
    stream_epoch: Option<u64>,
    connection_generation: u64,
    settings_generation: Option<u64>,
    source_generation: Option<u64>,
    treatment: Option<Treatment>,
    width: Option<u16>,
    height: Option<u16>,
    visibility: Visibility,
    correlation: CorrelationStatus,
    latency_us: Option<u64>,
    clock_uncertainty_us: Option<u64>,
    presented_at_unix_us: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServiceQualityResponse {
    schema_version: u8,
    limits: EvidenceLimits,
    clients: Vec<ClientReport>,
    combined: AggregateReport,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EvidenceLimits {
    clients: usize,
    samples_per_client: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientReport {
    client_id: Uuid,
    connection_generation: u64,
    samples: Vec<PresentationSample>,
    #[serde(flatten)]
    aggregate: AggregateReport,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct AggregateReport {
    sample_count: usize,
    exact_correlation: usize,
    unknown_correlation: usize,
    reconnects: u64,
    presentation_skips: u64,
    partitions: Vec<PartitionReport>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
struct PartitionKey {
    runtime_epoch: Uuid,
    stream_epoch: Option<u64>,
    connection_generation: u64,
    settings_generation: Option<u64>,
    treatment: Option<Treatment>,
    width: Option<u16>,
    height: Option<u16>,
    visibility: Visibility,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PartitionReport {
    #[serde(flatten)]
    key: PartitionKey,
    sample_count: usize,
    exact_correlation: usize,
    unknown_correlation: usize,
    unique_presented_frames: usize,
    unique_presented_cadence_hz: Option<f64>,
    latency_us: Option<Distribution>,
    clock_uncertainty_us: Option<Distribution>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct Distribution {
    min: u64,
    p50: u64,
    p95: u64,
    p99: u64,
    max: u64,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub(crate) enum ServiceQualityError {
    #[error("unsupported schema version")]
    UnsupportedSchema,
    #[error("runtime epoch does not match this process")]
    RuntimeMismatch,
    #[error("client has no active media connection")]
    UnknownClient,
    #[error("media connection generation is stale")]
    StaleConnection,
    #[error("presentation report is invalid")]
    InvalidReport,
}

impl ServiceQualityState {
    pub(crate) fn new(runtime_epoch: Uuid, correlation: CorrelationState) -> Self {
        Self {
            runtime_epoch,
            correlation,
            inner: Arc::new(Mutex::new(EvidenceStore::default())),
        }
    }

    pub(crate) fn begin_connection(
        &self,
        request: ConnectionRequest,
    ) -> Result<ConnectionResponse, ServiceQualityError> {
        self.validate_version_and_epoch(request.schema_version, request.runtime_epoch)?;
        let mut store = self.inner.lock().expect("service-quality mutex poisoned");
        let activity = store.next_activity();
        if let Some(client) = store
            .clients
            .iter_mut()
            .find(|client| client.client_id == request.client_id)
        {
            client.connection_generation = client
                .connection_generation
                .checked_add(1)
                .expect("connection generation exhausted");
            client.reconnects = client
                .reconnects
                .checked_add(1)
                .expect("reconnects exhausted");
            client.last_activity = activity;
            client.last_presented_frames = None;
            return Ok(client.connection_response());
        }

        if store.clients.len() == MAX_CLIENTS {
            let evicted = store
                .clients
                .iter()
                .enumerate()
                .min_by_key(|(_, client)| client.last_activity)
                .map(|(index, _)| index)
                .expect("client store is non-empty at capacity");
            store.clients.swap_remove(evicted);
        }
        let client = ClientEvidence {
            client_id: request.client_id,
            last_activity: activity,
            connection_generation: 1,
            reconnects: 0,
            presentation_skips: 0,
            last_presented_frames: None,
            samples: VecDeque::with_capacity(MAX_SAMPLES_PER_CLIENT),
        };
        let response = client.connection_response();
        store.clients.push(client);
        Ok(response)
    }

    pub(crate) fn record_batch(
        &self,
        batch: PresentationBatch,
        server_now_unix_us: u64,
    ) -> Result<(), ServiceQualityError> {
        self.validate_version_and_epoch(batch.schema_version, batch.runtime_epoch)?;
        if batch.connection_generation == 0
            || batch.samples.is_empty()
            || batch.samples.len() > MAX_REPORT_BATCH
        {
            return Err(ServiceQualityError::InvalidReport);
        }

        let prepared = batch
            .samples
            .into_iter()
            .map(|observation| {
                let report = PresentationReport {
                    #[cfg(test)]
                    client_id: batch.client_id,
                    runtime_epoch: batch.runtime_epoch,
                    connection_generation: batch.connection_generation,
                    observation,
                };
                self.prepare(report, server_now_unix_us)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut store = self.inner.lock().expect("service-quality mutex poisoned");
        let activity = store.next_activity();
        let client = store
            .clients
            .iter_mut()
            .find(|client| client.client_id == batch.client_id)
            .ok_or(ServiceQualityError::UnknownClient)?;
        if client.connection_generation != batch.connection_generation {
            return Err(ServiceQualityError::StaleConnection);
        }
        let mut previous = client
            .last_presented_frames
            .filter(|(generation, _)| *generation == batch.connection_generation)
            .map(|(_, presented_frames)| presented_frames);
        let mut additional_skips = 0_u64;
        for (presented_frames, _) in &prepared {
            if previous.is_some_and(|value| *presented_frames <= value) {
                return Err(ServiceQualityError::InvalidReport);
            }
            if let Some(value) = previous {
                additional_skips = additional_skips
                    .checked_add(*presented_frames - value - 1)
                    .expect("presentation skips exhausted");
            }
            previous = Some(*presented_frames);
        }
        client.presentation_skips = client
            .presentation_skips
            .checked_add(additional_skips)
            .expect("presentation skips exhausted");
        client.last_presented_frames =
            previous.map(|presented_frames| (batch.connection_generation, presented_frames));
        client.last_activity = activity;
        for (_, sample) in prepared {
            if client.samples.len() == MAX_SAMPLES_PER_CLIENT {
                client.samples.pop_front();
            }
            client.samples.push_back(sample);
        }
        Ok(())
    }

    #[cfg(test)]
    fn record(
        &self,
        report: PresentationReport,
        server_now_unix_us: u64,
    ) -> Result<(), ServiceQualityError> {
        self.record_batch(
            PresentationBatch {
                schema_version: SCHEMA_VERSION,
                client_id: report.client_id,
                runtime_epoch: report.runtime_epoch,
                connection_generation: report.connection_generation,
                samples: vec![report.observation],
            },
            server_now_unix_us,
        )
    }

    pub(crate) fn response(&self, client_id: Option<Uuid>) -> ServiceQualityResponse {
        let clients = {
            let store = self.inner.lock().expect("service-quality mutex poisoned");
            store
                .clients
                .iter()
                .filter(|client| client_id.is_none_or(|id| client.client_id == id))
                .cloned()
                .collect::<Vec<_>>()
        };
        let clients = clients.iter().collect::<Vec<_>>();
        response_for_clients(&clients)
    }

    fn prepare(
        &self,
        report: PresentationReport,
        server_now_unix_us: u64,
    ) -> Result<(u64, PresentationSample), ServiceQualityError> {
        let observation = report.observation;
        if observation.presented_frames == 0
            || observation.clock_uncertainty_us > MAX_CLOCK_UNCERTAINTY_US
            || observation
                .presented_at_unix_us
                .abs_diff(server_now_unix_us)
                > MAX_PRESENTATION_CLOCK_SKEW_US.saturating_add(observation.clock_uncertainty_us)
            || matches!(observation.stream_epoch, Some(0))
        {
            return Err(ServiceQualityError::InvalidReport);
        }
        let mapping = matches!(observation.correlation, ReportedCorrelation::Exact)
            .then(|| {
                observation
                    .stream_epoch
                    .zip(observation.rtp_timestamp)
                    .and_then(|(stream_epoch, rtp_timestamp)| {
                        self.correlation.lookup(stream_epoch, rtp_timestamp)
                    })
                    .filter(|mapping| mapping.runtime_epoch() == self.runtime_epoch)
            })
            .flatten();
        Ok((
            observation.presented_frames,
            PresentationSample::from_report(report, mapping),
        ))
    }

    fn validate_version_and_epoch(
        &self,
        schema_version: u8,
        runtime_epoch: Uuid,
    ) -> Result<(), ServiceQualityError> {
        if schema_version != SCHEMA_VERSION {
            return Err(ServiceQualityError::UnsupportedSchema);
        }
        if runtime_epoch != self.runtime_epoch {
            return Err(ServiceQualityError::RuntimeMismatch);
        }
        Ok(())
    }
}

impl EvidenceStore {
    fn next_activity(&mut self) -> u64 {
        self.activity = self
            .activity
            .checked_add(1)
            .expect("activity counter exhausted");
        self.activity
    }
}

impl ClientEvidence {
    const fn connection_response(&self) -> ConnectionResponse {
        ConnectionResponse {
            schema_version: SCHEMA_VERSION,
            client_id: self.client_id,
            connection_generation: self.connection_generation,
            reconnects: self.reconnects,
        }
    }
}

impl PresentationSample {
    fn from_report(report: PresentationReport, mapping: Option<CorrelationMapping>) -> Self {
        let observation = report.observation;
        let Some(mapping) = mapping else {
            return Self {
                runtime_epoch: report.runtime_epoch,
                stream_epoch: observation.stream_epoch,
                connection_generation: report.connection_generation,
                settings_generation: None,
                source_generation: None,
                treatment: None,
                width: None,
                height: None,
                visibility: observation.visibility,
                correlation: CorrelationStatus::Unknown,
                latency_us: None,
                clock_uncertainty_us: Some(observation.clock_uncertainty_us),
                presented_at_unix_us: observation.presented_at_unix_us,
            };
        };
        let (width, height) = mapping.dimensions();
        Self {
            runtime_epoch: mapping.runtime_epoch(),
            stream_epoch: Some(mapping.stream_epoch()),
            connection_generation: report.connection_generation,
            settings_generation: Some(mapping.settings_generation()),
            source_generation: Some(mapping.source_generation()),
            treatment: Some(mapping.treatment()),
            width: Some(width),
            height: Some(height),
            visibility: observation.visibility,
            correlation: CorrelationStatus::Exact,
            latency_us: observation
                .presented_at_unix_us
                .checked_sub(mapping.exposure_completed_at_unix_us()),
            clock_uncertainty_us: Some(observation.clock_uncertainty_us),
            presented_at_unix_us: observation.presented_at_unix_us,
        }
    }
}

fn response_for_clients(clients: &[&ClientEvidence]) -> ServiceQualityResponse {
    let reports = clients.iter().map(|client| client.report()).collect();
    let combined = aggregate(clients);
    ServiceQualityResponse {
        schema_version: SCHEMA_VERSION,
        limits: EvidenceLimits {
            clients: MAX_CLIENTS,
            samples_per_client: MAX_SAMPLES_PER_CLIENT,
        },
        clients: reports,
        combined,
    }
}

impl ClientEvidence {
    fn report(&self) -> ClientReport {
        ClientReport {
            client_id: self.client_id,
            connection_generation: self.connection_generation,
            samples: self.samples.iter().copied().collect(),
            aggregate: aggregate(&[self]),
        }
    }
}

fn aggregate(clients: &[&ClientEvidence]) -> AggregateReport {
    let mut report = AggregateReport::default();
    let mut partitions: BTreeMap<PartitionKey, Vec<PresentationSample>> = BTreeMap::new();
    for client in clients {
        report.reconnects = report.reconnects.saturating_add(client.reconnects);
        report.presentation_skips = report
            .presentation_skips
            .saturating_add(client.presentation_skips);
        for sample in &client.samples {
            report.sample_count += 1;
            match sample.correlation {
                CorrelationStatus::Exact => report.exact_correlation += 1,
                CorrelationStatus::Unknown => report.unknown_correlation += 1,
            }
            partitions
                .entry(PartitionKey::new(*sample))
                .or_default()
                .push(*sample);
        }
    }
    report.partitions = partitions
        .into_iter()
        .map(|(key, samples)| PartitionReport::new(key, &samples))
        .collect();
    report
}

impl PartitionKey {
    fn new(sample: PresentationSample) -> Self {
        Self {
            runtime_epoch: sample.runtime_epoch,
            stream_epoch: sample.stream_epoch,
            connection_generation: sample.connection_generation,
            settings_generation: sample.settings_generation,
            treatment: sample.treatment,
            width: sample.width,
            height: sample.height,
            visibility: sample.visibility,
        }
    }
}

impl PartitionReport {
    fn new(key: PartitionKey, samples: &[PresentationSample]) -> Self {
        let exact = samples
            .iter()
            .filter(|sample| sample.correlation == CorrelationStatus::Exact)
            .collect::<Vec<_>>();
        let unique = exact
            .iter()
            .filter_map(|sample| sample.source_generation)
            .collect::<BTreeSet<_>>();
        let cadence = cadence_hz(&exact, &unique);
        Self {
            key,
            sample_count: samples.len(),
            exact_correlation: exact.len(),
            unknown_correlation: samples.len() - exact.len(),
            unique_presented_frames: unique.len(),
            unique_presented_cadence_hz: cadence,
            latency_us: distribution(exact.iter().filter_map(|sample| sample.latency_us)),
            clock_uncertainty_us: distribution(
                samples
                    .iter()
                    .filter_map(|sample| sample.clock_uncertainty_us),
            ),
        }
    }
}

fn cadence_hz(samples: &[&PresentationSample], unique: &BTreeSet<u64>) -> Option<f64> {
    if unique.len() < 2 {
        return None;
    }
    let mut first_presentations = BTreeMap::<u64, u64>::new();
    for sample in samples {
        if let Some(generation) = sample.source_generation {
            first_presentations
                .entry(generation)
                .and_modify(|presented| *presented = (*presented).min(sample.presented_at_unix_us))
                .or_insert(sample.presented_at_unix_us);
        }
    }
    let min = first_presentations.values().min().copied()?;
    let max = first_presentations.values().max().copied()?;
    let elapsed = max.checked_sub(min)?;
    let intervals = u32::try_from(unique.len() - 1).expect("bounded evidence count fits u32");
    (elapsed > 0).then(|| f64::from(intervals) / Duration::from_micros(elapsed).as_secs_f64())
}

fn distribution(values: impl Iterator<Item = u64>) -> Option<Distribution> {
    let mut values = values.collect::<Vec<_>>();
    values.sort_unstable();
    Some(Distribution {
        min: *values.first()?,
        p50: nearest_rank(&values, 50),
        p95: nearest_rank(&values, 95),
        p99: nearest_rank(&values, 99),
        max: *values.last()?,
    })
}

fn nearest_rank(values: &[u64], percentile: usize) -> u64 {
    let rank = values.len().saturating_mul(percentile).div_ceil(100);
    values[rank.saturating_sub(1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::correlation::{CapturedFrameMetadata, CorrelationState};

    const RUNTIME: Uuid = Uuid::from_u128(1);

    fn state() -> (ServiceQualityState, CorrelationState) {
        let correlation = CorrelationState::new(RUNTIME);
        (
            ServiceQualityState::new(RUNTIME, correlation.clone()),
            correlation,
        )
    }

    fn connect(state: &ServiceQualityState, client_id: Uuid) -> ConnectionResponse {
        state
            .begin_connection(ConnectionRequest {
                schema_version: SCHEMA_VERSION,
                client_id,
                runtime_epoch: RUNTIME,
            })
            .expect("connection begins")
    }

    fn report(client_id: Uuid, generation: u64, ordinal: u64) -> PresentationReport {
        PresentationReport {
            client_id,
            runtime_epoch: RUNTIME,
            connection_generation: generation,
            observation: PresentationObservation {
                stream_epoch: None,
                rtp_timestamp: None,
                presented_frames: ordinal,
                presented_at_unix_us: 1_000_000 + ordinal * 50_000,
                clock_uncertainty_us: 1_000,
                visibility: Visibility::Visible,
                correlation: ReportedCorrelation::Unknown,
            },
        }
    }

    #[test]
    fn nearest_rank_uses_boundaries_without_interpolation() {
        let values = (1..=100).collect::<Vec<_>>();
        assert_eq!(
            distribution(values.into_iter()),
            Some(Distribution {
                min: 1,
                p50: 50,
                p95: 95,
                p99: 99,
                max: 100,
            })
        );
        assert_eq!(distribution([7].into_iter()).expect("distribution").p99, 7);
    }

    #[test]
    fn partition_key_uses_every_compatibility_dimension_but_not_source_identity() {
        let sample = PresentationSample {
            runtime_epoch: RUNTIME,
            stream_epoch: Some(1),
            connection_generation: 1,
            settings_generation: Some(1),
            source_generation: Some(1),
            treatment: Some(Treatment::Monochrome),
            width: Some(1_920),
            height: Some(1_080),
            visibility: Visibility::Visible,
            correlation: CorrelationStatus::Exact,
            latency_us: Some(10_000),
            clock_uncertainty_us: Some(1_000),
            presented_at_unix_us: 1_000_000,
        };
        let key = PartitionKey::new(sample);

        for incompatible in [
            PresentationSample {
                runtime_epoch: Uuid::from_u128(2),
                ..sample
            },
            PresentationSample {
                stream_epoch: Some(2),
                ..sample
            },
            PresentationSample {
                connection_generation: 2,
                ..sample
            },
            PresentationSample {
                settings_generation: Some(2),
                ..sample
            },
            PresentationSample {
                treatment: Some(Treatment::Colour),
                ..sample
            },
            PresentationSample {
                width: Some(1_280),
                ..sample
            },
            PresentationSample {
                height: Some(720),
                ..sample
            },
            PresentationSample {
                visibility: Visibility::Hidden,
                ..sample
            },
        ] {
            assert_ne!(PartitionKey::new(incompatible), key);
        }

        assert_eq!(
            PartitionKey::new(PresentationSample {
                source_generation: Some(2),
                ..sample
            }),
            key
        );
    }

    #[test]
    fn least_recently_active_client_is_evicted_at_sixteen() {
        let (state, _) = state();
        for id in 1..=16 {
            connect(&state, Uuid::from_u128(id));
        }
        state
            .record(report(Uuid::from_u128(1), 1, 1), 1_050_000)
            .expect("activity");
        connect(&state, Uuid::from_u128(17));
        let response = state.response(None);
        assert_eq!(response.clients.len(), MAX_CLIENTS);
        assert!(
            response
                .clients
                .iter()
                .any(|client| client.client_id == Uuid::from_u128(1))
        );
        assert!(
            !response
                .clients
                .iter()
                .any(|client| client.client_id == Uuid::from_u128(2))
        );
    }

    #[test]
    fn samples_are_bounded_and_clients_remain_separate() {
        let (state, _) = state();
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        connect(&state, first);
        connect(&state, second);
        for ordinal in 1..=513 {
            state
                .record(report(first, 1, ordinal), 1_000_000 + ordinal * 50_000)
                .expect("sample");
        }
        state
            .record(report(second, 1, 1), 1_050_000)
            .expect("sample");
        let first_report = state.response(Some(first));
        assert_eq!(
            first_report.clients[0].samples.len(),
            MAX_SAMPLES_PER_CLIENT
        );
        assert_eq!(
            first_report.clients[0].aggregate.sample_count,
            MAX_SAMPLES_PER_CLIENT
        );
        assert_eq!(
            state.response(Some(second)).clients[0]
                .aggregate
                .sample_count,
            1
        );
    }

    #[test]
    fn reconnects_partition_samples_and_reject_stale_reports() {
        let (state, _) = state();
        let client = Uuid::from_u128(1);
        connect(&state, client);
        state
            .record(report(client, 1, 1), 1_050_000)
            .expect("sample");
        let connection = connect(&state, client);
        assert_eq!(connection.connection_generation, 2);
        assert_eq!(connection.reconnects, 1);
        assert!(matches!(
            state.record(report(client, 1, 2), 1_100_000),
            Err(ServiceQualityError::StaleConnection)
        ));
        state
            .record(report(client, 2, 1), 1_050_000)
            .expect("sample");
        let response = state.response(Some(client));
        assert_eq!(response.clients[0].aggregate.reconnects, 1);
        assert_eq!(response.clients[0].aggregate.partitions.len(), 2);
    }

    #[test]
    fn exact_samples_are_server_validated_and_unknowns_have_no_invented_facts() {
        let (state, correlation) = state();
        let client = Uuid::from_u128(1);
        connect(&state, client);
        let stream = correlation.begin_stream();
        let input = correlation
            .submit(
                stream,
                CapturedFrameMetadata {
                    settings_generation: 7,
                    treatment: Treatment::Monochrome,
                    exposure_completed_at_unix_us: 1_000_000,
                },
                81,
                1_010_000,
                false,
            )
            .expect("submission");
        correlation.observe(stream, input, 55_000).expect("mapping");

        let mut exact = report(client, 1, 1);
        exact.observation.stream_epoch = Some(stream);
        exact.observation.rtp_timestamp = Some(55_000);
        exact.observation.correlation = ReportedCorrelation::Exact;
        exact.observation.presented_at_unix_us = 1_200_000;
        state.record(exact, 1_200_000).expect("exact report");
        state
            .record(report(client, 1, 2), 1_100_000)
            .expect("unknown report");
        let response = state.response(Some(client));
        let samples = &response.clients[0].samples;
        assert_eq!(samples[0].correlation, CorrelationStatus::Exact);
        assert_eq!(samples[0].source_generation, Some(81));
        assert_eq!(samples[0].latency_us, Some(200_000));
        assert_eq!(samples[1].correlation, CorrelationStatus::Unknown);
        assert_eq!(samples[1].source_generation, None);
        assert_eq!(samples[1].latency_us, None);
        assert_eq!(samples[1].clock_uncertainty_us, Some(1_000));
    }

    #[test]
    fn combined_evidence_deduplicates_source_generations_across_clients() {
        let (state, correlation) = state();
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        connect(&state, first);
        connect(&state, second);
        let stream = correlation.begin_stream();

        for source_generation in 1..=2 {
            let exposure_completed_at_unix_us = 1_000_000 + source_generation * 50_000;
            let input = correlation
                .submit(
                    stream,
                    CapturedFrameMetadata {
                        settings_generation: 7,
                        treatment: Treatment::Monochrome,
                        exposure_completed_at_unix_us,
                    },
                    source_generation,
                    exposure_completed_at_unix_us + 500,
                    false,
                )
                .expect("submission");
            let rtp_timestamp = u32::try_from(source_generation * 4_500).expect("RTP timestamp");
            correlation
                .observe(stream, input, rtp_timestamp)
                .expect("mapping");
            for client in [first, second] {
                let mut exact = report(client, 1, source_generation);
                exact.observation.correlation = ReportedCorrelation::Exact;
                exact.observation.stream_epoch = Some(stream);
                exact.observation.rtp_timestamp = Some(rtp_timestamp);
                exact.observation.presented_at_unix_us = exposure_completed_at_unix_us + 100_000;
                let presented_at_unix_us = exact.observation.presented_at_unix_us;
                state
                    .record(exact, presented_at_unix_us)
                    .expect("exact report");
            }
        }

        let combined = state.response(None).combined;
        assert_eq!(combined.sample_count, 4);
        assert_eq!(combined.partitions.len(), 1);
        assert_eq!(combined.partitions[0].unique_presented_frames, 2);
        assert_eq!(
            combined.partitions[0].unique_presented_cadence_hz,
            Some(20.0)
        );
    }

    #[test]
    fn exact_partition_reports_nearest_rank_latency_and_unique_cadence() {
        let (state, correlation) = state();
        let client = Uuid::from_u128(1);
        connect(&state, client);
        let stream = correlation.begin_stream();
        for ordinal in 1..=100_u64 {
            let exposure_completed_at_unix_us = 1_000_000 + ordinal * 50_000;
            let input = correlation
                .submit(
                    stream,
                    CapturedFrameMetadata {
                        settings_generation: 7,
                        treatment: Treatment::Monochrome,
                        exposure_completed_at_unix_us,
                    },
                    ordinal,
                    exposure_completed_at_unix_us + 500,
                    false,
                )
                .expect("submission");
            let rtp_timestamp = u32::try_from(ordinal * 4_500).expect("RTP timestamp");
            correlation
                .observe(stream, input, rtp_timestamp)
                .expect("mapping");
            let presented_at_unix_us = exposure_completed_at_unix_us + ordinal * 1_000;
            let mut exact = report(client, 1, ordinal);
            exact.observation.correlation = ReportedCorrelation::Exact;
            exact.observation.stream_epoch = Some(stream);
            exact.observation.rtp_timestamp = Some(rtp_timestamp);
            exact.observation.presented_at_unix_us = presented_at_unix_us;
            state
                .record(exact, presented_at_unix_us)
                .expect("exact report");
        }

        let response = state.response(Some(client));
        let aggregate = &response.clients[0].aggregate;
        assert_eq!(aggregate.sample_count, 100);
        assert_eq!(aggregate.exact_correlation, 100);
        assert_eq!(aggregate.partitions.len(), 1);
        let partition = &aggregate.partitions[0];
        assert_eq!(partition.unique_presented_frames, 100);
        assert_eq!(
            partition.latency_us,
            Some(Distribution {
                min: 1_000,
                p50: 50_000,
                p95: 95_000,
                p99: 99_000,
                max: 100_000,
            })
        );
        let cadence = partition
            .unique_presented_cadence_hz
            .expect("cadence with multiple unique frames");
        assert!((cadence - 19.607_843).abs() < 0.000_001, "{cadence}");
    }

    #[test]
    fn settings_and_treatment_changes_create_compatible_partitions() {
        let (state, correlation) = state();
        let client = Uuid::from_u128(1);
        connect(&state, client);
        let stream = correlation.begin_stream();
        for (index, (settings_generation, treatment)) in
            [(7, Treatment::Monochrome), (8, Treatment::Colour)]
                .into_iter()
                .enumerate()
        {
            let input = correlation
                .submit(
                    stream,
                    CapturedFrameMetadata {
                        settings_generation,
                        treatment,
                        exposure_completed_at_unix_us: 1_000_000,
                    },
                    u64::try_from(index + 1).expect("source generation"),
                    1_000_500,
                    false,
                )
                .expect("submission");
            let rtp_timestamp = u32::try_from((index + 1) * 4_500).expect("RTP timestamp");
            correlation
                .observe(stream, input, rtp_timestamp)
                .expect("mapping");
            let mut exact = report(
                client,
                1,
                u64::try_from(index + 1).expect("presented frames"),
            );
            exact.observation.correlation = ReportedCorrelation::Exact;
            exact.observation.stream_epoch = Some(stream);
            exact.observation.rtp_timestamp = Some(rtp_timestamp);
            state.record(exact, 1_050_000).expect("exact report");
        }

        let partitions = &state.response(Some(client)).clients[0].aggregate.partitions;
        assert_eq!(partitions.len(), 2);
        assert_eq!(partitions[0].key.settings_generation, Some(7));
        assert_eq!(partitions[0].key.treatment, Some(Treatment::Monochrome));
        assert_eq!(partitions[1].key.settings_generation, Some(8));
        assert_eq!(partitions[1].key.treatment, Some(Treatment::Colour));
    }

    #[test]
    fn visibility_partitions_and_callback_gaps_count_as_presentation_skips() {
        let (state, _) = state();
        let client = Uuid::from_u128(1);
        connect(&state, client);
        state
            .record(report(client, 1, 1), 1_050_000)
            .expect("visible");
        let mut hidden = report(client, 1, 4);
        hidden.observation.visibility = Visibility::Hidden;
        state.record(hidden, 1_200_000).expect("hidden");
        let response = state.response(Some(client));
        assert_eq!(response.clients[0].aggregate.presentation_skips, 2);
        assert_eq!(response.clients[0].aggregate.partitions.len(), 2);
    }
}
