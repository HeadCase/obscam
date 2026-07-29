use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Deserialize)]
pub struct BrowserConnection {
    pub schema_version: u8,
    pub client_id: String,
    pub runtime_epoch: String,
    pub stream_epoch: u64,
}

impl BrowserConnection {
    pub fn is_valid(&self) -> bool {
        self.schema_version == SCHEMA_VERSION
            && valid_identifier(&self.client_id)
            && valid_identifier(&self.runtime_epoch)
            && self.stream_epoch > 0
    }
}

impl BrowserPresentation {
    /// Validates untrusted browser telemetry before it reaches correlation state.
    pub fn is_valid(&self) -> bool {
        self.schema_version == SCHEMA_VERSION
            && valid_identifier(&self.client_id)
            && valid_identifier(&self.runtime_epoch)
            && self.stream_epoch > 0
            && self.expected_display_unix_ms.is_finite()
            && self.expected_display_unix_ms > 0.0
            && self.clock_uncertainty_ms.is_none_or(|uncertainty| {
                uncertainty.is_finite() && (0.0..=60_000.0).contains(&uncertainty)
            })
            && self.presented_frames > 0
            && (1..=8192).contains(&self.width)
            && (1..=8192).contains(&self.height)
            && matches!(self.visibility_state.as_str(), "visible" | "hidden")
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 80 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
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
    pub whep: WhepEndpoint,
    pub capture: Option<CaptureProgress>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WhepEndpoint {
    pub port: u16,
    pub path: String,
}

#[cfg(test)]
mod tests {
    use super::{BrowserPresentation, RuntimeDescription, SCHEMA_VERSION, WhepEndpoint};

    #[test]
    fn runtime_media_contract_has_no_host() {
        let runtime = RuntimeDescription {
            schema_version: SCHEMA_VERSION,
            runtime_epoch: "epoch".into(),
            stream_epoch: 1,
            whep: WhepEndpoint {
                port: 18_889,
                path: "/obscam/whep".into(),
            },
            capture: None,
        };

        let value = serde_json::to_value(runtime).expect("runtime contract must serialize");
        assert_eq!(value["whep"]["port"], 18_889);
        assert_eq!(value["whep"]["path"], "/obscam/whep");
        assert!(value["whep"].get("host").is_none());
        assert!(value.get("whep_url").is_none());
    }

    fn presentation() -> BrowserPresentation {
        BrowserPresentation {
            schema_version: SCHEMA_VERSION,
            client_id: "abc123".into(),
            runtime_epoch: "epoch123".into(),
            stream_epoch: 1,
            rtp_timestamp: Some(42),
            expected_display_unix_ms: 1_000.0,
            clock_uncertainty_ms: Some(0.5),
            presented_frames: 1,
            width: 1920,
            height: 1080,
            visibility_state: "visible".into(),
        }
    }

    #[test]
    fn validates_complete_browser_presentation() {
        assert!(presentation().is_valid());
    }

    #[test]
    fn rejects_nonsensical_browser_presentation_values() {
        let mut value = presentation();
        value.expected_display_unix_ms = f64::NAN;
        assert!(!value.is_valid());
        value = presentation();
        value.clock_uncertainty_ms = Some(-1.0);
        assert!(!value.is_valid());
        value = presentation();
        value.width = 0;
        assert!(!value.is_valid());
        value = presentation();
        value.visibility_state = "prerender".into();
        assert!(!value.is_valid());
        value = presentation();
        value.client_id = "invalid-id".into();
        assert!(!value.is_valid());
    }
}
