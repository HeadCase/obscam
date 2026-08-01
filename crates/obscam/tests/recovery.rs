use std::time::{Duration, Instant};

use obscam::{
    AuthorityState, CameraRecoveryBackoff, CameraSettings, ComponentReadiness, Config,
    ExposureWatchdog, RuntimeState, Treatment, ValidationFailure, ValidationWindow, WatchdogAction,
    validate_processing_output,
};
use uuid::Uuid;

#[test]
fn camera_retry_is_immediate_then_jittered_and_capped_near_thirty_seconds() {
    let mut backoff = CameraRecoveryBackoff::new();
    let expected = [0, 1, 2, 4, 8, 15, 30, 30];

    for seconds in expected {
        assert_eq!(
            backoff.next_delay_with_jitter(0),
            Duration::from_secs(seconds)
        );
    }

    let mut jittered = CameraRecoveryBackoff::new();
    assert_eq!(jittered.next_delay_with_jitter(250), Duration::ZERO);
    assert_eq!(
        jittered.next_delay_with_jitter(250),
        Duration::from_millis(1_250)
    );
}

#[test]
fn three_responsible_validation_failures_within_ten_seconds_trigger_recovery() {
    let mut failures = ValidationWindow::new(Duration::from_secs(10));

    assert!(!failures.record(ValidationFailure::InvalidDimensions, Duration::ZERO));
    assert!(!failures.record(
        ValidationFailure::InvalidBufferLength,
        Duration::from_secs(1)
    ));
    assert!(!failures.record(ValidationFailure::InvalidDimensions, Duration::from_secs(5)));
    assert!(failures.record(
        ValidationFailure::InvalidDimensions,
        Duration::from_secs(10)
    ));
    assert_eq!(failures.count(ValidationFailure::InvalidDimensions), 3);
    assert_eq!(failures.count(ValidationFailure::InvalidBufferLength), 1);
}

#[test]
fn failures_outside_the_validation_window_do_not_accumulate() {
    let mut failures = ValidationWindow::new(Duration::from_secs(10));

    assert!(!failures.record(ValidationFailure::InvalidGeneration, Duration::ZERO));
    assert!(!failures.record(
        ValidationFailure::InvalidGeneration,
        Duration::from_secs(11)
    ));
    assert!(!failures.record(
        ValidationFailure::InvalidGeneration,
        Duration::from_secs(20)
    ));
    assert_eq!(failures.count(ValidationFailure::InvalidGeneration), 2);
}

#[test]
fn exposure_watchdog_cancels_after_exposure_plus_two_seconds_then_terminates() {
    let watchdog = ExposureWatchdog::new(Duration::from_millis(500));

    assert_eq!(
        watchdog.action_at(Duration::from_millis(2_499)),
        WatchdogAction::Wait
    );
    assert_eq!(
        watchdog.action_at(Duration::from_millis(2_500)),
        WatchdogAction::Cancel
    );
    assert_eq!(
        watchdog.action_at(Duration::from_millis(3_499)),
        WatchdogAction::Cancel
    );
    assert_eq!(
        watchdog.action_at(Duration::from_millis(3_500)),
        WatchdogAction::Terminate
    );
}

#[test]
fn camera_recovery_revokes_authority_and_discards_unapplied_intent() {
    let config = Config::parse("127.0.0.1:8080", "8889", "/obscam/whep").expect("configuration");
    let runtime = RuntimeState::with_readiness(
        Uuid::from_u128(1),
        &config,
        ComponentReadiness::Ready,
        ComponentReadiness::Ready,
        ComponentReadiness::Ready,
    );
    runtime.settings().mark_camera_ready();
    let authority = runtime.authority();
    let now = Instant::now();
    let grant = authority.take(now);
    authority
        .accept(&grant.credentials(), now, || {
            runtime
                .settings()
                .accept(CameraSettings::new(20, 200, Treatment::Colour).expect("settings"))
        })
        .expect("current authority")
        .expect("camera ready");

    runtime.begin_camera_recovery();

    assert_eq!(authority.snapshot().state(), AuthorityState::Unheld);
    assert!(runtime.settings().snapshot().pending().is_none());
    assert_eq!(runtime.settings().snapshot().applied().generation(), 0);
}

#[test]
fn processing_output_must_match_native_i420_and_the_claimed_source_generation() {
    let native_i420 = 1920 * 1080 * 3 / 2;
    assert!(validate_processing_output(7, 7, 1920, 1080, native_i420));
    assert!(!validate_processing_output(7, 6, 1920, 1080, native_i420));
    assert!(!validate_processing_output(7, 7, 1280, 720, native_i420));
    assert!(!validate_processing_output(
        7,
        7,
        1920,
        1080,
        native_i420 - 1
    ));
}
