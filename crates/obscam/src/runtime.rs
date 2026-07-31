use std::sync::{Arc, RwLock};

use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{
    AuthorityGate, Config, SettingsController, config::WhepPath, correlation::CorrelationState,
    service_quality::ServiceQualityState,
};

pub(crate) const SCHEMA_VERSION: u8 = 1;

/// Fixed-size, RAM-only facts exposed to browser clients.
#[derive(Clone, Debug)]
pub struct RuntimeState {
    snapshot: Arc<RwLock<RuntimeSnapshot>>,
    authority: AuthorityGate,
    settings: SettingsController,
    correlation: CorrelationState,
    service_quality: ServiceQualityState,
    lifecycle_updates: broadcast::Sender<()>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSnapshot {
    schema_version: u8,
    pub(crate) runtime_epoch: Uuid,
    media: MediaDescriptor,
    pub(crate) components: Components,
    latest_frame: Option<LatestFrame>,
    capture: Option<CaptureProgress>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LifecycleSnapshot {
    pub(crate) runtime_epoch: Uuid,
    pub(crate) components: Components,
    pub(crate) recovery: Option<RecoveryComponent>,
    pub(crate) capture: Option<CaptureProgress>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoveryComponent {
    Capture,
    Encoder,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureProgress {
    settings_generation: u64,
    exposure_ms: u32,
    started_at_unix_us: u64,
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
        let correlation = CorrelationState::new(runtime_epoch);
        let (lifecycle_updates, _) = broadcast::channel(32);
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
                capture: None,
            })),
            authority: AuthorityGate::new(),
            settings: SettingsController::new(config.default_settings()),
            service_quality: ServiceQualityState::new(runtime_epoch, correlation.clone()),
            correlation,
            lifecycle_updates,
        }
    }

    /// Returns the epoch that fences all runtime-local facts.
    #[must_use]
    pub fn runtime_epoch(&self) -> Uuid {
        self.snapshot().runtime_epoch
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
        {
            let mut snapshot = self.snapshot.write().expect("runtime state lock poisoned");
            snapshot.components.capture =
                ComponentStatus::from_readiness(readiness, UnavailableReason::NoCameraSource);
            if readiness == ComponentReadiness::Unavailable {
                snapshot.capture = None;
            }
        }
        self.notify_lifecycle();
    }

    pub(crate) fn set_encoder_readiness(&self, readiness: ComponentReadiness) {
        self.snapshot
            .write()
            .expect("runtime state lock poisoned")
            .components
            .encoder = ComponentStatus::from_readiness(readiness, UnavailableReason::NoFrame);
        self.notify_lifecycle();
    }

    /// Records the authoritative start of the currently progressing exposure.
    ///
    /// # Panics
    ///
    /// Panics if the runtime-state lock was poisoned by another thread.
    pub fn capture_started(
        &self,
        settings_generation: u64,
        exposure_ms: u32,
        started_at_unix_us: u64,
    ) {
        self.snapshot
            .write()
            .expect("runtime state lock poisoned")
            .capture = Some(CaptureProgress {
            settings_generation,
            exposure_ms,
            started_at_unix_us,
        });
        self.notify_lifecycle();
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

    pub(crate) fn correlation(&self) -> CorrelationState {
        self.correlation.clone()
    }

    pub(crate) fn service_quality(&self) -> ServiceQualityState {
        self.service_quality.clone()
    }

    pub(crate) fn lifecycle_snapshot(&self) -> LifecycleSnapshot {
        let snapshot = self.snapshot();
        LifecycleSnapshot {
            runtime_epoch: snapshot.runtime_epoch,
            recovery: snapshot.components.recovery(),
            components: snapshot.components,
            capture: snapshot.capture,
        }
    }

    pub(crate) fn subscribe_lifecycle(&self) -> broadcast::Receiver<()> {
        self.lifecycle_updates.subscribe()
    }

    fn notify_lifecycle(&self) {
        let _ = self.lifecycle_updates.send(());
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

impl Components {
    fn recovery(&self) -> Option<RecoveryComponent> {
        if matches!(self.capture, ComponentStatus::Unavailable { .. }) {
            Some(RecoveryComponent::Capture)
        } else if matches!(self.encoder, ComponentStatus::Unavailable { .. }) {
            Some(RecoveryComponent::Encoder)
        } else {
            None
        }
    }
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
