//! Production `ObsCam` service.

mod assets;
mod authority;
mod config;
mod encoder;
mod latest;
mod monochrome;
mod pipeline;
mod runtime;
mod service;

pub use authority::{
    AuthorityCredentials, AuthorityGate, AuthorityGrant, AuthorityLease, AuthorityRejection,
    AuthoritySnapshot, AuthorityState,
};
pub use config::{CameraSourceKind, Config, ConfigError};
pub use encoder::FfmpegEncoder;
pub use latest::{LatestFrameMailbox, PublishedFrame};
pub use monochrome::{MonochromeFrame, MonochromeProcessor};
pub use pipeline::MediaPipeline;
pub use runtime::{ComponentReadiness, RuntimeState};
pub use service::serve;
