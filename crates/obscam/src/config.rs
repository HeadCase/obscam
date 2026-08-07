use std::{env, net::SocketAddr, num::NonZeroU16};

use serde::Serialize;
use thiserror::Error;

use crate::{CameraSettings, Treatment};

const MAX_WHEP_PATH_BYTES: usize = 256;

/// Validated host configuration required to start the HTTP service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    bind_address: SocketAddr,
    whep_port: NonZeroU16,
    whep_path: WhepPath,
    camera_source: CameraSourceKind,
    default_settings: CameraSettings,
}

impl Config {
    /// Loads host configuration from the environment, using local-only defaults.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when an environment value is not valid UTF-8 or
    /// when the complete configuration is invalid.
    pub fn from_environment() -> Result<Self, ConfigError> {
        let bind_address = environment_value("OBSCAM_BIND_ADDRESS", "0.0.0.0:8080")?;
        let whep_port = environment_value("OBSCAM_WHEP_PORT", "8889")?;
        let whep_path = environment_value("OBSCAM_WHEP_PATH", "/obscam/whep")?;
        let camera_source = environment_value("OBSCAM_CAMERA_SOURCE", "production")?;
        let exposure_ms = environment_value("OBSCAM_DEFAULT_EXPOSURE_MS", "500")?;
        let gain = environment_value("OBSCAM_DEFAULT_GAIN", "100")?;
        let treatment = environment_value("OBSCAM_DEFAULT_TREATMENT", "monochrome")?;
        Self::parse_with_defaults(
            &bind_address,
            &whep_port,
            &whep_path,
            &camera_source,
            &exposure_ms,
            &gain,
            &treatment,
        )
    }

    /// Parses and validates the complete startup configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when any value cannot be normalized into the
    /// required bind address or hostless media descriptor.
    pub fn parse(
        bind_address: &str,
        whep_port: &str,
        whep_path: &str,
    ) -> Result<Self, ConfigError> {
        Self::parse_with_defaults(
            bind_address,
            whep_port,
            whep_path,
            "production",
            "500",
            "100",
            "monochrome",
        )
    }

    /// Parses the complete startup configuration with an explicit camera boundary.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when any host or camera-source value is invalid.
    pub fn parse_with_camera_source(
        bind_address: &str,
        whep_port: &str,
        whep_path: &str,
        camera_source: &str,
    ) -> Result<Self, ConfigError> {
        Self::parse_with_defaults(
            bind_address,
            whep_port,
            whep_path,
            camera_source,
            "500",
            "100",
            "monochrome",
        )
    }

    /// Parses host, camera-source, and restart-default configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when any value is not one of the accepted choices.
    #[allow(clippy::too_many_arguments)]
    pub fn parse_with_defaults(
        bind_address: &str,
        whep_port: &str,
        whep_path: &str,
        camera_source: &str,
        exposure_ms: &str,
        gain: &str,
        treatment: &str,
    ) -> Result<Self, ConfigError> {
        let bind_address = bind_address
            .parse()
            .map_err(|_| ConfigError::InvalidBindAddress)?;
        let whep_port = whep_port
            .parse::<u16>()
            .ok()
            .and_then(NonZeroU16::new)
            .ok_or(ConfigError::InvalidWhepPort)?;
        let whep_path = WhepPath::parse(whep_path)?;
        let camera_source = match camera_source {
            "production" => CameraSourceKind::Production,
            "deterministic" => CameraSourceKind::Deterministic,
            _ => return Err(ConfigError::InvalidCameraSource),
        };
        let default_settings = CameraSettings::new(
            exposure_ms
                .parse()
                .map_err(|_| ConfigError::InvalidDefaultSettings)?,
            gain.parse()
                .map_err(|_| ConfigError::InvalidDefaultSettings)?,
            treatment
                .parse::<Treatment>()
                .map_err(|_| ConfigError::InvalidDefaultSettings)?,
        )
        .map_err(|_| ConfigError::InvalidDefaultSettings)?;

        Ok(Self {
            bind_address,
            whep_port,
            whep_path,
            camera_source,
            default_settings,
        })
    }

    /// Address on which the browser-facing HTTP service listens.
    #[must_use]
    pub const fn bind_address(&self) -> SocketAddr {
        self.bind_address
    }

    /// Port used by the browser to construct an origin-aware WHEP URL.
    #[must_use]
    pub const fn whep_port(&self) -> u16 {
        self.whep_port.get()
    }

    /// Hostless path used by the browser to construct an origin-aware WHEP URL.
    #[must_use]
    pub fn whep_path(&self) -> &str {
        self.whep_path.as_str()
    }

    /// Explicit camera hardware boundary selected for this process.
    #[must_use]
    pub const fn camera_source(&self) -> CameraSourceKind {
        self.camera_source
    }

    /// Complete tuple restored at every runtime start.
    #[must_use]
    pub const fn default_settings(&self) -> CameraSettings {
        self.default_settings
    }

    pub(crate) fn whep_path_value(&self) -> WhepPath {
        self.whep_path.clone()
    }
}

/// Camera boundary selected at startup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CameraSourceKind {
    /// Exact production ASI662MC owner.
    Production,
    /// Feature-gated deterministic development and acceptance substitute.
    Deterministic,
}

/// A bounded path that cannot supply its own media authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct WhepPath(Box<str>);

impl WhepPath {
    fn parse(value: &str) -> Result<Self, ConfigError> {
        if value.len() > MAX_WHEP_PATH_BYTES {
            return Err(ConfigError::WhepPathTooLong);
        }
        if !is_hostless_absolute_path(value) {
            return Err(ConfigError::InvalidWhepPath);
        }
        Ok(Self(value.into()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_hostless_absolute_path(value: &str) -> bool {
    value.starts_with('/')
        && !value.starts_with("//")
        && !value.contains(['?', '#'])
        && !value.chars().any(char::is_whitespace)
}

fn environment_value(name: &'static str, default: &str) -> Result<String, ConfigError> {
    match env::var(name) {
        Ok(value) => Ok(value),
        Err(env::VarError::NotPresent) => Ok(default.to_owned()),
        Err(env::VarError::NotUnicode(_)) => Err(ConfigError::InvalidEnvironmentValue(name)),
    }
}

/// Reason startup configuration was rejected.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ConfigError {
    /// Environment input cannot be represented as UTF-8 and therefore cannot be validated.
    #[error("{0} must contain valid UTF-8")]
    InvalidEnvironmentValue(&'static str),
    /// HTTP bind address is not a socket address.
    #[error("OBSCAM_BIND_ADDRESS must be an IP socket address")]
    InvalidBindAddress,
    /// WHEP port is absent, non-numeric, or zero.
    #[error("OBSCAM_WHEP_PORT must be between 1 and 65535")]
    InvalidWhepPort,
    /// WHEP path is not a hostless absolute path.
    #[error("OBSCAM_WHEP_PATH must be a hostless absolute path")]
    InvalidWhepPath,
    /// WHEP path would exceed the fixed runtime-state bound.
    #[error("OBSCAM_WHEP_PATH must not exceed 256 bytes")]
    WhepPathTooLong,
    /// Camera source is not one of the two explicit boundary selections.
    #[error("OBSCAM_CAMERA_SOURCE must be production or deterministic")]
    InvalidCameraSource,
    /// Restart camera defaults are not a complete accepted settings tuple.
    #[error("default exposure, gain, and treatment must be accepted choices")]
    InvalidDefaultSettings,
}
