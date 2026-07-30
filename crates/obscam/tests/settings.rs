use obscam::{CameraSettings, SettingsController, SettingsFailure, Treatment};

const EXPOSURES_MS: [u32; 14] = [
    10, 20, 50, 100, 200, 300, 500, 1_000, 2_000, 5_000, 10_000, 15_000, 20_000, 30_000,
];

#[test]
fn every_curated_exposure_and_gain_detent_is_accepted() {
    for exposure_ms in EXPOSURES_MS {
        for gain in (0..=600).step_by(50) {
            let settings = CameraSettings::new(exposure_ms, gain, Treatment::Monochrome)
                .expect("curated tuple");
            assert_eq!(settings.exposure_ms(), exposure_ms);
            assert_eq!(settings.gain(), gain);
        }
    }
}

#[test]
fn values_between_curated_detents_are_rejected() {
    assert!(CameraSettings::new(30, 100, Treatment::Monochrome).is_err());
    assert!(CameraSettings::new(500, 75, Treatment::Colour).is_err());
    assert!(CameraSettings::new(500, 650, Treatment::Colour).is_err());
}

#[test]
fn newer_pending_tuple_supersedes_the_old_target_without_advancing_applied_state() {
    let controller = SettingsController::new(CameraSettings::default());
    let first =
        controller.accept(CameraSettings::new(100, 200, Treatment::Colour).expect("first tuple"));
    let second = controller
        .accept(CameraSettings::new(20, 400, Treatment::Monochrome).expect("second tuple"));

    assert_eq!(first.generation(), 1);
    assert_eq!(second.generation(), 2);
    assert_eq!(controller.snapshot().applied().generation(), 0);
    assert_eq!(controller.claim_latest(), Some(second));
    assert!(controller.claim_latest().is_none());
}

#[test]
fn recovery_fails_accepted_work_and_preserves_the_last_fully_applied_tuple() {
    let defaults = CameraSettings::default();
    let controller = SettingsController::new(defaults);
    let target =
        controller.accept(CameraSettings::new(50, 300, Treatment::Colour).expect("target tuple"));
    assert_eq!(controller.claim_latest(), Some(target));

    controller.fail_recovery();

    let snapshot = controller.snapshot();
    assert_eq!(snapshot.applied().settings(), defaults);
    assert!(snapshot.pending().is_none());
    assert_eq!(
        controller.last_failure(),
        Some((target.generation(), SettingsFailure::Recovery))
    );
}

#[cfg(feature = "camera-substitute")]
#[test]
fn accepting_settings_interrupts_the_active_deterministic_exposure() {
    use zwo_asi::{
        CameraSource, CaptureError, CapturePlan, DeterministicCamera, DeterministicScenario,
        Settings,
    };

    let mut camera =
        DeterministicCamera::connect(DeterministicScenario::new([CapturePlan::Frame {
            additional_delay_us: 240_000,
        }]))
        .expect("camera present");
    camera
        .configure(Settings::new(10_000, 100).expect("camera settings"))
        .expect("configure");
    camera.start().expect("start");
    let controller = SettingsController::new(CameraSettings::default());
    controller.install_interrupter(camera.interrupter());

    let _target =
        controller.accept(CameraSettings::new(20, 200, Treatment::Colour).expect("target"));

    assert_eq!(
        camera.capture_next(100).err(),
        Some(CaptureError::Interrupted)
    );
    assert_eq!(
        controller
            .claim_latest()
            .map(obscam::SettingsTarget::generation),
        Some(1)
    );
}
