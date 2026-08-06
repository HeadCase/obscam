use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{
    BUFFER_COUNT, CameraError, CameraSource, CaptureError, CaptureInterrupter, FRAME_BYTES,
    FrameGeneration, HEIGHT, MAX_CAPTURE_WAIT_MS, Settings, WIDTH,
};

/// One deterministic outcome at the camera hardware boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapturePlan {
    /// Complete a valid generation after its exposure and an optional extra delay.
    Frame {
        /// Additional virtual delay after the configured exposure.
        additional_delay_us: u64,
    },
    /// Return one timeout without advancing the source generation.
    Timeout,
    /// Disconnect the source and stop capture.
    Disconnect,
    /// Complete a frame with invalid dimensions and reject it at the boundary.
    MalformedDimensions {
        /// Reported width.
        width: usize,
        /// Reported height.
        height: usize,
    },
    /// Complete a frame with an invalid byte length and reject it at the boundary.
    MalformedLength {
        /// Reported byte length.
        length: usize,
    },
    /// Return a non-timeout SDK capture failure.
    Sdk {
        /// Normalized vendor result code.
        code: i32,
    },
    /// Complete a frame with invalid source-generation metadata.
    MalformedGeneration {
        /// Invalid generation value.
        generation: u64,
    },
}

/// RAM-only deterministic camera behavior supplied by development or acceptance code.
#[derive(Clone, Debug)]
pub struct DeterministicScenario {
    camera_present: bool,
    plans: VecDeque<CapturePlan>,
}

impl DeterministicScenario {
    /// Creates a present camera with the supplied ordered capture outcomes.
    pub fn new(plans: impl IntoIterator<Item = CapturePlan>) -> Self {
        Self {
            camera_present: true,
            plans: plans.into_iter().collect(),
        }
    }

    /// Controls whether connecting observes the deterministic camera.
    #[must_use]
    pub const fn camera_present(mut self, camera_present: bool) -> Self {
        self.camera_present = camera_present;
        self
    }
}

/// Full-resolution deterministic RAW8 source for development and acceptance only.
pub struct DeterministicCamera {
    plans: VecDeque<CapturePlan>,
    buffers: [Box<[u8]>; BUFFER_COUNT],
    next_buffer: usize,
    generation: u64,
    settings: Option<Settings>,
    capturing: bool,
    connected: bool,
    interrupted: Arc<AtomicBool>,
    pending_delay_us: Option<u64>,
}

impl DeterministicCamera {
    /// Connects to an explicitly configured deterministic scenario.
    ///
    /// # Errors
    ///
    /// Returns [`CameraError::IdentityCount`] when the scenario models camera absence.
    pub fn connect(scenario: DeterministicScenario) -> Result<Self, CameraError> {
        if !scenario.camera_present {
            return Err(CameraError::IdentityCount { found: 0 });
        }
        Ok(Self {
            plans: scenario.plans,
            buffers: std::array::from_fn(|_| vec![0; FRAME_BYTES].into_boxed_slice()),
            next_buffer: 0,
            generation: 0,
            settings: None,
            capturing: false,
            connected: true,
            interrupted: Arc::new(AtomicBool::new(false)),
            pending_delay_us: None,
        })
    }

    /// Restores a scripted disconnected source to an unconfigured stopped state.
    pub fn recover(&mut self) {
        self.connected = true;
        self.settings = None;
        self.capturing = false;
        self.interrupted.store(false, Ordering::Release);
        self.pending_delay_us = None;
    }

    fn begin_plan(&mut self) -> Result<(), CaptureError> {
        let plan = self.plans.pop_front().unwrap_or(CapturePlan::Frame {
            additional_delay_us: 0,
        });
        match plan {
            CapturePlan::Frame {
                additional_delay_us,
            } => {
                let exposure_us = self
                    .settings
                    .expect("capturing source has settings")
                    .exposure_us();
                self.pending_delay_us = Some(
                    u64::try_from(exposure_us)
                        .expect("validated exposure is positive")
                        .saturating_add(additional_delay_us),
                );
                Ok(())
            }
            CapturePlan::Timeout => Err(CaptureError::Timeout),
            CapturePlan::Disconnect => {
                self.connected = false;
                self.capturing = false;
                Err(CaptureError::Disconnected)
            }
            CapturePlan::MalformedDimensions { width, height } => {
                Err(CaptureError::MalformedDimensions { width, height })
            }
            CapturePlan::MalformedLength { length } => {
                Err(CaptureError::MalformedLength { length })
            }
            CapturePlan::Sdk { code } => Err(CaptureError::Sdk { code }),
            CapturePlan::MalformedGeneration { generation } => {
                Err(CaptureError::MalformedGeneration {
                    previous: self.generation,
                    received: generation,
                })
            }
        }
    }
}

impl CameraSource for DeterministicCamera {
    fn interrupter(&self) -> CaptureInterrupter {
        CaptureInterrupter(Arc::clone(&self.interrupted))
    }

    fn configure(&mut self, settings: Settings) -> Result<(), CameraError> {
        if !self.connected {
            return Err(CameraError::InvalidState {
                operation: "configure disconnected camera",
            });
        }
        if self.capturing {
            return Err(CameraError::InvalidState {
                operation: "configure while capturing",
            });
        }
        self.settings = Some(settings);
        self.pending_delay_us = None;
        Ok(())
    }

    fn apply_live_settings(&mut self, settings: Settings) -> Result<(), CameraError> {
        if !self.connected || !self.capturing {
            return Err(CameraError::InvalidState {
                operation: "apply live settings while stopped",
            });
        }
        self.interrupted.store(false, Ordering::Release);
        self.settings = Some(settings);
        self.pending_delay_us = None;
        Ok(())
    }

    fn start(&mut self) -> Result<(), CameraError> {
        if !self.connected || self.settings.is_none() || self.capturing {
            return Err(CameraError::InvalidState { operation: "start" });
        }
        self.capturing = true;
        self.interrupted.store(false, Ordering::Release);
        self.pending_delay_us = None;
        Ok(())
    }

    fn capture_next(&mut self, wait_ms: i32) -> Result<FrameGeneration<'_>, CaptureError> {
        if !self.capturing {
            return Err(CaptureError::NotCapturing);
        }
        if self.interrupted.swap(false, Ordering::AcqRel) {
            return Err(CaptureError::Interrupted);
        }
        if self.pending_delay_us.is_none() {
            self.begin_plan()?;
        }
        let waited_us = u64::try_from(wait_ms.clamp(0, MAX_CAPTURE_WAIT_MS))
            .expect("clamped wait is non-negative")
            * 1_000;
        let remaining = self.pending_delay_us.expect("plan initialized");
        if waited_us < remaining {
            self.pending_delay_us = Some(remaining - waited_us);
            return Err(CaptureError::Timeout);
        }
        self.pending_delay_us = None;
        let generation = self.generation.saturating_add(1);
        let buffer = &mut self.buffers[self.next_buffer];
        fill_pattern(
            buffer,
            generation,
            self.settings.expect("capturing source has settings").gain(),
        );
        if self.interrupted.swap(false, Ordering::AcqRel) {
            return Err(CaptureError::Interrupted);
        }
        self.generation = generation;
        self.next_buffer = (self.next_buffer + 1) % BUFFER_COUNT;
        Ok(FrameGeneration {
            generation: self.generation,
            data: buffer,
            sdk_drops: 0,
        })
    }

    fn stop(&mut self) -> Result<(), CameraError> {
        self.capturing = false;
        self.pending_delay_us = None;
        Ok(())
    }
}

fn fill_pattern(buffer: &mut [u8], generation: u64, gain: i64) {
    let gain_boost = u8::try_from(gain / 20).expect("validated gain boost fits u8");
    let generation_offset =
        u8::try_from(generation.saturating_sub(1) % 31).expect("generation marker fits u8");
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let coordinate = u8::try_from(((x / 64) * 3 + (y / 64) * 5) % 31)
                .expect("coordinate marker fits u8");
            let bayer_base = match (x % 2, y % 2) {
                (0, 0) => 160,
                (1, 1) => 32,
                _ => 96,
            };
            buffer[y * WIDTH + x] = bayer_base + coordinate + gain_boost + generation_offset;
        }
    }

    for x in 0..128 {
        let bit = (generation >> (x / 2)) & 1;
        let marker = if bit == 1 {
            240 - generation_offset
        } else {
            16 + generation_offset
        };
        buffer[x] = marker;
        buffer[WIDTH + x] = marker;
    }
}
