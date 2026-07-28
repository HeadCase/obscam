use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Treatment {
    Mono,
    Colour,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FrameMapping {
    pub schema_version: u8,
    pub runtime_epoch: String,
    pub stream_epoch: u64,
    pub rtp_timestamp: u32,
    pub source_generation: u64,
    pub settings_generation: u64,
    pub treatment: Treatment,
    pub exposure_completed_unix_ns: u128,
    pub submitted_unix_ns: u128,
}

#[derive(Clone, Debug)]
pub struct SubmittedFrame {
    pub source_generation: u64,
    pub settings_generation: u64,
    pub treatment: Treatment,
    pub exposure_completed_unix_ns: u128,
    pub submitted_unix_ns: u128,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CaptureProgress {
    pub schema_version: u8,
    pub runtime_epoch: String,
    pub settings_generation: u64,
    pub source_generation: u64,
    pub exposure_us: i64,
    pub capture_started_unix_ns: u128,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BrowserPresentation {
    pub schema_version: u8,
    pub client_id: String,
    pub runtime_epoch: String,
    pub stream_epoch: u64,
    pub rtp_timestamp: Option<u32>,
    pub expected_display_unix_ms: f64,
    pub clock_uncertainty_ms: Option<f64>,
    pub presented_frames: u64,
    pub width: u32,
    pub height: u32,
    pub visibility_state: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrelationStatus {
    Correlated,
    UnknownNoRtpTimestamp,
    UnknownMissing,
    UnknownAmbiguous,
    UnknownEpoch,
}

#[derive(Clone, Debug, Serialize)]
pub struct CorrelationResult {
    pub status: CorrelationStatus,
    pub observed_rtp_timestamp: Option<u32>,
    pub frame: Option<FrameMapping>,
    pub exposure_end_to_visible_ms: Option<f64>,
    pub clock_uncertainty_ms: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuntimeDescription {
    pub schema_version: u8,
    pub runtime_epoch: String,
    pub stream_epoch: u64,
    pub whep_url: String,
    pub capture: Option<CaptureProgress>,
}
