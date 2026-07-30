//! Production `ObsCam` service.

mod assets;
mod authority;
mod colour;
mod config;
mod encoder;
mod latest;
mod monochrome;
mod pipeline;
mod runtime;
mod service;
mod settings;

pub use authority::{
    AuthorityCredentials, AuthorityGate, AuthorityGrant, AuthorityLease, AuthorityRejection,
    AuthoritySnapshot, AuthorityState,
};
pub use colour::ColourProcessor;
pub use config::{CameraSourceKind, Config, ConfigError};
pub use encoder::FfmpegEncoder;
pub use latest::{LatestFrameMailbox, PublishedFrame};
pub use monochrome::{MonochromeFrame, MonochromeProcessor, ProcessedFrame};
pub use pipeline::{MediaPipeline, SettingsTransition, apply_pending_settings};
pub use runtime::{ComponentReadiness, RuntimeState};
pub use service::serve;
pub use settings::{
    CameraSettings, EXPOSURE_CHOICES_MS, SettingsController, SettingsError, SettingsFailure,
    SettingsSnapshot, SettingsTarget, SettingsUnavailable, Treatment,
};
