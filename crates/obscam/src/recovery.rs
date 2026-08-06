use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

use zwo_asi::CaptureInterrupter;

const CAMERA_RETRY_SECONDS: [u64; 8] = [0, 1, 2, 4, 8, 15, 30, 30];
const MAX_CAMERA_RETRY_DELAY: Duration = Duration::from_secs(30);
const WATCHDOG_CANCEL_MARGIN: Duration = Duration::from_secs(2);
const WATCHDOG_TERMINATE_MARGIN: Duration = Duration::from_secs(3);

/// Validates that processed bytes represent the claimed native I420 source generation.
#[must_use]
pub const fn validate_processing_output(
    source_generation: u64,
    output_generation: u64,
    width: usize,
    height: usize,
    length: usize,
) -> bool {
    output_generation == source_generation
        && width == zwo_asi::WIDTH
        && height == zwo_asi::HEIGHT
        && length == zwo_asi::WIDTH * zwo_asi::HEIGHT * 3 / 2
}

/// Indefinite camera-reopen retry policy with a bounded production delay.
#[derive(Debug, Default)]
pub struct CameraRecoveryBackoff {
    failures: usize,
}

impl CameraRecoveryBackoff {
    /// Creates a retry sequence whose first attempt is immediate.
    #[must_use]
    pub const fn new() -> Self {
        Self { failures: 0 }
    }

    /// Returns the next retry delay with signed millisecond jitter.
    pub fn next_delay_with_jitter(&mut self, jitter_ms: i64) -> Duration {
        let index = self.failures.min(CAMERA_RETRY_SECONDS.len() - 1);
        let base = Duration::from_secs(CAMERA_RETRY_SECONDS[index]);
        self.failures = self.failures.saturating_add(1);
        if base.is_zero() {
            return base;
        }
        if jitter_ms >= 0 {
            base.saturating_add(Duration::from_millis(jitter_ms.unsigned_abs()))
                .min(MAX_CAMERA_RETRY_DELAY)
        } else {
            base.saturating_sub(Duration::from_millis(jitter_ms.unsigned_abs()))
        }
    }

    /// Restarts the schedule after a trustworthy frame completes.
    pub const fn reset(&mut self) {
        self.failures = 0;
    }
}

/// Validation categories with independent counters and recovery thresholds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationFailure {
    /// The source did not report native dimensions.
    InvalidDimensions,
    /// The source or processor returned the wrong byte length.
    InvalidBufferLength,
    /// Source or processed generation identity was not current and monotonic.
    InvalidGeneration,
    /// Processed output did not satisfy the native I420 contract.
    InvalidProcessingOutput,
}

impl ValidationFailure {
    const COUNT: usize = 4;

    const fn index(self) -> usize {
        match self {
            Self::InvalidDimensions => 0,
            Self::InvalidBufferLength => 1,
            Self::InvalidGeneration => 2,
            Self::InvalidProcessingOutput => 3,
        }
    }
}

/// Sliding per-category validation-failure window.
#[derive(Debug)]
pub struct ValidationWindow {
    window: Duration,
    failures: [VecDeque<Duration>; ValidationFailure::COUNT],
}

impl ValidationWindow {
    /// Creates an empty validation window.
    #[must_use]
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            failures: std::array::from_fn(|_| VecDeque::with_capacity(3)),
        }
    }

    /// Records one failure and reports whether its category reached three events.
    pub fn record(&mut self, failure: ValidationFailure, now: Duration) -> bool {
        let failures = &mut self.failures[failure.index()];
        while failures
            .front()
            .is_some_and(|oldest| now.saturating_sub(*oldest) > self.window)
        {
            failures.pop_front();
        }
        failures.push_back(now);
        failures.len() >= 3
    }

    /// Returns the current bounded count for one category.
    #[must_use]
    pub fn count(&self, failure: ValidationFailure) -> usize {
        self.failures[failure.index()].len()
    }

    /// Clears all component-local failure history after recovery.
    pub fn reset(&mut self) {
        for failures in &mut self.failures {
            failures.clear();
        }
    }
}

/// Action required by an armed exposure watchdog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchdogAction {
    /// The exposure remains inside its permitted completion window.
    Wait,
    /// Capture must receive an interruption request.
    Cancel,
    /// Rust must terminate so the external supervisor can restore ownership safely.
    Terminate,
}

/// Exposure-relative watchdog deadlines independent of wall-clock time.
#[derive(Clone, Copy, Debug)]
pub struct ExposureWatchdog {
    cancel_at: Duration,
    terminate_at: Duration,
}

impl ExposureWatchdog {
    /// Arms the watchdog for one authoritative exposure duration.
    #[must_use]
    pub fn new(exposure: Duration) -> Self {
        let cancel_at = exposure.saturating_add(WATCHDOG_CANCEL_MARGIN);
        Self {
            cancel_at,
            terminate_at: cancel_at.saturating_add(WATCHDOG_TERMINATE_MARGIN),
        }
    }

    /// Returns the action required at the supplied elapsed exposure time.
    #[must_use]
    pub fn action_at(self, elapsed: Duration) -> WatchdogAction {
        if elapsed >= self.terminate_at {
            WatchdogAction::Terminate
        } else if elapsed >= self.cancel_at {
            WatchdogAction::Cancel
        } else {
            WatchdogAction::Wait
        }
    }
}

#[derive(Debug)]
struct ArmedExposure {
    token: u64,
    started: Instant,
    watchdog: ExposureWatchdog,
    interrupter: CaptureInterrupter,
    cancellation_requested: bool,
}

#[derive(Debug, Default)]
struct WatchdogState {
    next_token: u64,
    armed: Option<ArmedExposure>,
}

/// Process-level watchdog monitor kept separate from the potentially blocked SDK owner.
#[derive(Clone, Debug)]
pub(crate) struct CaptureWatchdogMonitor {
    shared: Arc<(Mutex<WatchdogState>, Condvar)>,
}

impl CaptureWatchdogMonitor {
    pub(crate) fn spawn() -> std::io::Result<Self> {
        let shared = Arc::new((Mutex::new(WatchdogState::default()), Condvar::new()));
        let worker = Arc::clone(&shared);
        thread::Builder::new()
            .name("obscam-capture-watchdog".into())
            .spawn(move || watchdog_worker(&worker))?;
        Ok(Self { shared })
    }

    pub(crate) fn arm(&self, exposure: Duration, interrupter: CaptureInterrupter) -> u64 {
        let (state, changed) = &*self.shared;
        let mut state = state.lock().expect("capture watchdog mutex poisoned");
        state.next_token = state.next_token.saturating_add(1);
        let token = state.next_token;
        state.armed = Some(ArmedExposure {
            token,
            started: Instant::now(),
            watchdog: ExposureWatchdog::new(exposure),
            interrupter,
            cancellation_requested: false,
        });
        changed.notify_all();
        token
    }

    pub(crate) fn complete(&self, token: u64) {
        let (state, changed) = &*self.shared;
        let mut state = state.lock().expect("capture watchdog mutex poisoned");
        if state
            .armed
            .as_ref()
            .is_some_and(|armed| armed.token == token)
        {
            state.armed = None;
            changed.notify_all();
        }
    }

    pub(crate) fn cancellation_requested(&self, token: u64) -> bool {
        self.shared
            .0
            .lock()
            .expect("capture watchdog mutex poisoned")
            .armed
            .as_ref()
            .is_some_and(|armed| armed.token == token && armed.cancellation_requested)
    }
}

fn watchdog_worker(shared: &(Mutex<WatchdogState>, Condvar)) -> ! {
    let (state, changed) = shared;
    loop {
        let mut state = state.lock().expect("capture watchdog mutex poisoned");
        while state.armed.is_none() {
            state = changed
                .wait(state)
                .expect("capture watchdog mutex poisoned while idle");
        }
        let armed = state.armed.as_ref().expect("armed exposure exists");
        let token = armed.token;
        let elapsed = armed.started.elapsed();
        let action = armed.watchdog.action_at(elapsed);
        match action {
            WatchdogAction::Wait => {
                let remaining = armed.watchdog.cancel_at.saturating_sub(elapsed);
                drop(
                    changed
                        .wait_timeout(state, remaining)
                        .expect("capture watchdog mutex poisoned while waiting"),
                );
            }
            WatchdogAction::Cancel => {
                let interrupter = armed.interrupter.clone();
                let remaining = armed.watchdog.terminate_at.saturating_sub(elapsed);
                state
                    .armed
                    .as_mut()
                    .expect("armed exposure exists")
                    .cancellation_requested = true;
                interrupter.interrupt();
                drop(
                    changed
                        .wait_timeout(state, remaining)
                        .expect("capture watchdog mutex poisoned after cancellation"),
                );
            }
            WatchdogAction::Terminate => {
                tracing::error!(
                    token,
                    "camera capture ignored watchdog cancellation; terminating"
                );
                std::process::exit(70);
            }
        }
    }
}
