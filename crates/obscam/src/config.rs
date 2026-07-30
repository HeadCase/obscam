use std::{env, net::SocketAddr, num::NonZeroU16};

use serde::Serialize;
use thiserror::Error;

const MAX_WHEP_PATH_BYTES: usize = 256;

/// Validated host configuration required to start the HTTP service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    bind_address: SocketAddr,
    whep_port: NonZeroU16,
    whep_path: WhepPath,
    camera_source: CameraSourceSelection,
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
        Self::parse_with_camera_source(&bind_address, &whep_port, &whep_path, &camera_source)
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
        Self::parse_with_camera_source(bind_address, whep_port, whep_path, "production")
    }

    /// Parses complete startup configuration with an explicit camera source.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when a value cannot be normalized or when the
    /// requested source is not present in this build.
    pub fn parse_with_camera_source(
        bind_address: &str,
        whep_port: &str,
        whep_path: &str,
        camera_source: &str,
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
        let camera_source = CameraSourceSelection::parse(camera_source)?;

        Ok(Self {
            bind_address,
            whep_port,
            whep_path,
            camera_source,
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

    /// Explicitly selected camera hardware boundary.
    #[must_use]
    pub const fn camera_source(&self) -> CameraSourceSelection {
        self.camera_source
    }

    pub(crate) fn whep_path_value(&self) -> WhepPath {
        self.whep_path.clone()
    }
}

/// Camera hardware boundary selected for this process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CameraSourceSelection {
    /// Exact production ASI662MC owner.
    Production,
    /// Feature-gated deterministic development and acceptance source.
    #[cfg(feature = "camera-substitute")]
    Deterministic,
}

impl CameraSourceSelection {
    fn parse(value: &str) -> Result<Self, ConfigError> {
        match value {
            "production" => Ok(Self::Production),
            "deterministic" => {
                #[cfg(feature = "camera-substitute")]
                {
                    Ok(Self::Deterministic)
                }
                #[cfg(not(feature = "camera-substitute"))]
                {
                    Err(ConfigError::CameraSubstituteUnavailable)
                }
            }
            _ => Err(ConfigError::InvalidCameraSource),
        }
    }
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

    fn as_str(&self) -> &str {
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
    /// Camera source selection is not one of the supported explicit names.
    #[error("OBSCAM_CAMERA_SOURCE must be production or deterministic")]
    InvalidCameraSource,
    /// The deterministic source was requested from a production build.
    #[error("OBSCAM_CAMERA_SOURCE=deterministic requires the camera-substitute feature")]
    CameraSubstituteUnavailable,
}
