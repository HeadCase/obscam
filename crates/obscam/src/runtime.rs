use std::sync::{Arc, RwLock};

use serde::Serialize;
use uuid::Uuid;

use crate::{AuthorityGate, Config, SettingsController, config::WhepPath};

pub(crate) const SCHEMA_VERSION: u8 = 1;

/// Fixed-size, RAM-only facts exposed to browser clients.
#[derive(Clone, Debug)]
pub struct RuntimeState {
    snapshot: Arc<RwLock<RuntimeSnapshot>>,
    authority: AuthorityGate,
    settings: SettingsController,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSnapshot {
    schema_version: u8,
    pub(crate) runtime_epoch: Uuid,
    media: MediaDescriptor,
    pub(crate) components: Components,
    latest_frame: Option<LatestFrame>,
}

impl RuntimeState {
    /// Creates the truthful state used before capture or media components are ready.
    #[must_use]
    pub fn unavailable(runtime_epoch: Uuid, config: &Config) -> Self {
        Self::with_readiness(
            runtime_epoch,
            config,
            ComponentReadiness::Unavailable,
            ComponentReadiness::Unavailable,
            ComponentReadiness::Unavailable,
        )
    }

    /// Creates a snapshot with independently reported component readiness.
    #[must_use]
    pub fn with_readiness(
        runtime_epoch: Uuid,
        config: &Config,
        capture: ComponentReadiness,
        encoder: ComponentReadiness,
        relay: ComponentReadiness,
    ) -> Self {
        Self {
            snapshot: Arc::new(RwLock::new(RuntimeSnapshot {
                schema_version: SCHEMA_VERSION,
                runtime_epoch,
                media: MediaDescriptor {
                    whep_port: config.whep_port(),
                    whep_path: config.whep_path_value(),
                },
                components: Components {
                    capture: ComponentStatus::from_readiness(
                        capture,
                        UnavailableReason::NoCameraSource,
                    ),
                    encoder: ComponentStatus::from_readiness(encoder, UnavailableReason::NoFrame),
                    relay: ComponentStatus::from_readiness(relay, UnavailableReason::NotObserved),
                },
                latest_frame: None,
            })),
            authority: AuthorityGate::new(),
            settings: SettingsController::new(config.default_settings()),
        }
    }

    pub(crate) fn snapshot(&self) -> RuntimeSnapshot {
        self.snapshot
            .read()
            .expect("runtime state lock poisoned")
            .clone()
    }

    pub(crate) fn set_capture_readiness(&self, readiness: ComponentReadiness) {
        if readiness == ComponentReadiness::Unavailable {
            self.authority.revoke();
        }
        self.snapshot
            .write()
            .expect("runtime state lock poisoned")
            .components
            .capture =
            ComponentStatus::from_readiness(readiness, UnavailableReason::NoCameraSource);
    }

    pub(crate) fn set_encoder_readiness(&self, readiness: ComponentReadiness) {
        self.snapshot
            .write()
            .expect("runtime state lock poisoned")
            .components
            .encoder = ComponentStatus::from_readiness(readiness, UnavailableReason::NoFrame);
    }

    /// Returns the one runtime-local mutation authority gate.
    #[must_use]
    pub fn authority(&self) -> AuthorityGate {
        self.authority.clone()
    }

    /// Returns the one runtime-local settings coordinator.
    #[must_use]
    pub fn settings(&self) -> SettingsController {
        self.settings.clone()
    }

    /// Revokes authority when the camera backend restarts or runtime recovery begins.
    pub fn revoke_authority(&self) {
        self.authority.revoke();
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaDescriptor {
    whep_port: u16,
    whep_path: WhepPath,
}

/// Readiness of one independently supervised runtime component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentReadiness {
    /// The component is able to perform its role.
    Ready,
    /// The component cannot currently perform its role.
    Unavailable,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Components {
    capture: ComponentStatus,
    encoder: ComponentStatus,
    relay: ComponentStatus,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum ComponentStatus {
    Ready,
    Unavailable { reason: UnavailableReason },
}

impl ComponentStatus {
    const fn from_readiness(
        readiness: ComponentReadiness,
        unavailable_reason: UnavailableReason,
    ) -> Self {
        match readiness {
            ComponentReadiness::Ready => Self::Ready,
            ComponentReadiness::Unavailable => Self::Unavailable {
                reason: unavailable_reason,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum UnavailableReason {
    NoCameraSource,
    NoFrame,
    NotObserved,
}

#[derive(Clone, Debug, Serialize)]
struct LatestFrame {
    source_generation: u64,
}
