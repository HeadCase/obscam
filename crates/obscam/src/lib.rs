//! Production `ObsCam` service.

mod assets;
mod config;
mod runtime;
mod service;

pub use config::{CameraSourceSelection, Config, ConfigError};
pub use runtime::{ComponentReadiness, RuntimeState};
pub use service::serve;
