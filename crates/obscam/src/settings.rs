use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::broadcast;
use zwo_asi::CaptureInterrupter;

/// The fourteen operator-facing exposure choices, in milliseconds.
pub const EXPOSURE_CHOICES_MS: [u32; 14] = [
    10, 20, 50, 100, 200, 300, 500, 1_000, 2_000, 5_000, 10_000, 15_000, 20_000, 30_000,
];

/// One of the two deliberately neutral production image treatments.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Treatment {
    /// Full-resolution neutral luminance with neutral chroma.
    Monochrome,
    /// Full-resolution neutral colour.
    Colour,
}

impl Treatment {
    /// Stable configuration and control-contract spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Monochrome => "monochrome",
            Self::Colour => "colour",
        }
    }
}

impl std::str::FromStr for Treatment {
    type Err = SettingsError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "monochrome" => Ok(Self::Monochrome),
            "colour" => Ok(Self::Colour),
            _ => Err(SettingsError),
        }
    }
}

/// A complete, validated camera and treatment mutation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraSettings {
    exposure_ms: u32,
    gain: u16,
    treatment: Treatment,
}

impl CameraSettings {
    /// Validates a tuple against the operator-facing detents.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError`] unless exposure and gain are exact accepted choices.
    pub fn new(exposure_ms: u32, gain: u16, treatment: Treatment) -> Result<Self, SettingsError> {
        if !EXPOSURE_CHOICES_MS.contains(&exposure_ms) || gain > 600 || !gain.is_multiple_of(50) {
            return Err(SettingsError);
        }
        Ok(Self {
            exposure_ms,
            gain,
            treatment,
        })
    }

    /// Exposure duration in whole milliseconds.
    #[must_use]
    pub const fn exposure_ms(self) -> u32 {
        self.exposure_ms
    }

    /// Camera gain detent.
    #[must_use]
    pub const fn gain(self) -> u16 {
        self.gain
    }

    /// Selected neutral treatment.
    #[must_use]
    pub const fn treatment(self) -> Treatment {
        self.treatment
    }

    pub(crate) fn camera_settings(self) -> zwo_asi::Settings {
        zwo_asi::Settings::new(i64::from(self.exposure_ms) * 1_000, i64::from(self.gain))
            .expect("operator settings are inside the camera owner's validated range")
    }
}

/// A settings tuple contains a value outside the fixed operator choices.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("camera settings are not an accepted complete tuple")]
pub struct SettingsError;

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            exposure_ms: 500,
            gain: 100,
            treatment: Treatment::Monochrome,
        }
    }
}

/// One accepted settings target with its reserved semantic generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsTarget {
    generation: u64,
    settings: CameraSettings,
}

impl SettingsTarget {
    /// Reserved settings generation.
    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }

    /// Complete tuple associated with this generation.
    #[must_use]
    pub const fn settings(self) -> CameraSettings {
        self.settings
    }
}

/// Why accepted settings could not become applied.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsFailure {
    /// A newer complete tuple replaced pending work.
    Superseded,
    /// Camera recovery began before the target became fully applied.
    Recovery,
}

/// Authoritative accepted and applied settings state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    applied: SettingsTarget,
    pending: Option<SettingsTarget>,
}

impl SettingsSnapshot {
    /// Last fully applied tuple.
    #[must_use]
    pub const fn applied(self) -> SettingsTarget {
        self.applied
    }

    /// Newest accepted or currently applying target.
    #[must_use]
    pub const fn pending(self) -> Option<SettingsTarget> {
        self.pending
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum SettingsEvent {
    Accepted,
    Applied(SettingsTarget),
    Failed {
        generation: u64,
        reason: SettingsFailure,
    },
}

#[derive(Debug)]
struct SettingsState {
    applied: SettingsTarget,
    pending: Option<SettingsTarget>,
    applying: Option<SettingsTarget>,
    next_generation: u64,
    last_failure: Option<(u64, SettingsFailure)>,
}

/// Bounded RAM-only coordinator between the authority gate and camera owner.
#[derive(Clone, Debug)]
pub struct SettingsController {
    state: Arc<Mutex<SettingsState>>,
    interrupter: Arc<Mutex<Option<CaptureInterrupter>>>,
    updates: broadcast::Sender<SettingsEvent>,
}

impl SettingsController {
    /// Creates a coordinator whose generation zero is the restart tuple.
    #[must_use]
    pub fn new(defaults: CameraSettings) -> Self {
        let (updates, _) = broadcast::channel(32);
        Self {
            state: Arc::new(Mutex::new(SettingsState {
                applied: SettingsTarget {
                    generation: 0,
                    settings: defaults,
                },
                pending: None,
                applying: None,
                next_generation: 0,
                last_failure: None,
            })),
            interrupter: Arc::new(Mutex::new(None)),
            updates,
        }
    }

    /// Reserves a generation and replaces any older work that has not started applying.
    ///
    /// # Panics
    ///
    /// Panics if an internal mutex is poisoned or the generation space is exhausted.
    #[must_use]
    pub fn accept(&self, settings: CameraSettings) -> SettingsTarget {
        let (target, superseded) = {
            let mut state = self.state.lock().expect("settings mutex poisoned");
            state.next_generation = state
                .next_generation
                .checked_add(1)
                .expect("settings generation exhausted");
            let target = SettingsTarget {
                generation: state.next_generation,
                settings,
            };
            let superseded = state.pending.replace(target);
            if let Some(previous) = superseded {
                state.last_failure = Some((previous.generation, SettingsFailure::Superseded));
            }
            (target, superseded)
        };
        if let Some(previous) = superseded {
            let _ = self.updates.send(SettingsEvent::Failed {
                generation: previous.generation,
                reason: SettingsFailure::Superseded,
            });
        }
        let _ = self.updates.send(SettingsEvent::Accepted);
        if let Some(interrupter) = self
            .interrupter
            .lock()
            .expect("settings interrupter mutex poisoned")
            .as_ref()
        {
            interrupter.interrupt();
        }
        target
    }

    /// Claims the newest pending target for the single camera-owner thread.
    ///
    /// # Panics
    ///
    /// Panics if an internal mutex is poisoned.
    #[must_use]
    pub fn claim_latest(&self) -> Option<SettingsTarget> {
        let mut state = self.state.lock().expect("settings mutex poisoned");
        if state.applying.is_some() {
            return None;
        }
        let target = state.pending.take()?;
        state.applying = Some(target);
        Some(target)
    }

    /// Marks the claimed target fully applied after the media boundary advances.
    ///
    /// # Panics
    ///
    /// Panics if the target was not claimed or an internal mutex is poisoned.
    pub fn mark_applied(&self, target: SettingsTarget) {
        let applied = {
            let mut state = self.state.lock().expect("settings mutex poisoned");
            assert_eq!(
                state.applying,
                Some(target),
                "only claimed settings may apply"
            );
            state.applying = None;
            state.applied = target;
            target
        };
        let _ = self.updates.send(SettingsEvent::Applied(applied));
    }

    /// Fails all accepted work while retaining the previous fully applied tuple.
    ///
    /// # Panics
    ///
    /// Panics if an internal mutex is poisoned.
    pub fn fail_recovery(&self) {
        let failed = {
            let mut state = self.state.lock().expect("settings mutex poisoned");
            let mut failed = Vec::with_capacity(2);
            if let Some(target) = state.applying.take() {
                failed.push(target);
            }
            if let Some(target) = state.pending.take() {
                failed.push(target);
            }
            if let Some(target) = failed.last() {
                state.last_failure = Some((target.generation, SettingsFailure::Recovery));
            }
            failed
        };
        for target in failed {
            let _ = self.updates.send(SettingsEvent::Failed {
                generation: target.generation,
                reason: SettingsFailure::Recovery,
            });
        }
    }

    /// Installs the current camera owner's thread-safe capture interruption handle.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn install_interrupter(&self, interrupter: CaptureInterrupter) {
        *self
            .interrupter
            .lock()
            .expect("settings interrupter mutex poisoned") = Some(interrupter);
    }

    /// Returns the authoritative bounded state.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn snapshot(&self) -> SettingsSnapshot {
        let state = self.state.lock().expect("settings mutex poisoned");
        SettingsSnapshot {
            applied: state.applied,
            pending: state.applying.or(state.pending),
        }
    }

    /// Last target failure, retained for deterministic diagnostics and tests.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn last_failure(&self) -> Option<(u64, SettingsFailure)> {
        self.state
            .lock()
            .expect("settings mutex poisoned")
            .last_failure
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<SettingsEvent> {
        self.updates.subscribe()
    }
}
