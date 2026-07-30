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
    let controller = ready_controller(CameraSettings::default());
    let first = controller
        .accept(CameraSettings::new(100, 200, Treatment::Colour).expect("first tuple"))
        .expect("camera ready");
    let second = controller
        .accept(CameraSettings::new(20, 400, Treatment::Monochrome).expect("second tuple"))
        .expect("camera ready");

    assert_eq!(first.generation(), 1);
    assert_eq!(second.generation(), 2);
    assert_eq!(controller.snapshot().applied().generation(), 0);
    assert_eq!(controller.claim_latest(), Some(second));
    assert!(controller.claim_latest().is_none());
}

#[test]
fn recovery_fails_accepted_work_and_preserves_the_last_fully_applied_tuple() {
    let defaults = CameraSettings::default();
    let controller = ready_controller(defaults);
    let target = controller
        .accept(CameraSettings::new(50, 300, Treatment::Colour).expect("target tuple"))
        .expect("camera ready");
    assert_eq!(controller.claim_latest(), Some(target));

    controller.begin_recovery();

    let snapshot = controller.snapshot();
    assert_eq!(snapshot.applied().settings(), defaults);
    assert!(snapshot.pending().is_none());
    assert_eq!(
        controller.last_failure(),
        Some((target.generation(), SettingsFailure::Recovery))
    );
}

#[test]
fn newest_pending_target_remains_authoritative_while_an_older_target_applies() {
    let controller = ready_controller(CameraSettings::default());
    let applying = controller
        .accept(CameraSettings::new(100, 100, Treatment::Monochrome).expect("applying"))
        .expect("camera ready");
    assert_eq!(controller.claim_latest(), Some(applying));
    let pending = controller
        .accept(CameraSettings::new(20, 350, Treatment::Colour).expect("pending"))
        .expect("camera ready");

    assert_eq!(controller.snapshot().pending(), Some(pending));
}

#[test]
fn recovery_rejects_new_intent_until_the_previous_tuple_is_restored() {
    let controller = ready_controller(CameraSettings::default());
    controller.begin_recovery();

    assert!(
        controller
            .accept(CameraSettings::new(20, 100, Treatment::Monochrome).expect("tuple"))
            .is_err()
    );
}

#[cfg(feature = "camera-substitute")]
#[test]
fn accepting_settings_interrupts_the_active_deterministic_exposure() {
    use zwo_asi::{
        CameraSource, CaptureError, DeterministicCamera, DeterministicScenario, Settings,
    };

    let mut camera =
        DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
    camera
        .configure(Settings::new(10_000, 100).expect("camera settings"))
        .expect("configure");
    camera.start().expect("start");
    let controller = ready_controller(CameraSettings::default());
    controller.install_interrupter(camera.interrupter());

    let _target = controller
        .accept(CameraSettings::new(20, 200, Treatment::Colour).expect("target"))
        .expect("camera ready");

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

#[cfg(feature = "camera-substitute")]
#[test]
fn pending_tuple_runs_the_complete_camera_and_generation_transition() {
    use obscam::{SettingsTransition, apply_pending_settings};
    use zwo_asi::{CameraSource, DeterministicCamera, DeterministicScenario, Settings};

    let mut camera =
        DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
    camera
        .configure(Settings::new(500_000, 100).expect("defaults"))
        .expect("configure");
    camera.start().expect("start");
    let controller = ready_controller(CameraSettings::default());
    controller.install_interrupter(camera.interrupter());
    let target = controller
        .accept(CameraSettings::new(20, 200, Treatment::Colour).expect("target"))
        .expect("camera ready");
    assert_eq!(
        camera.capture_next(100).err(),
        Some(zwo_asi::CaptureError::Interrupted)
    );
    let mut boundaries = Vec::new();

    let transition = apply_pending_settings(&mut camera, &controller, |generation| {
        boundaries.push(generation);
    })
    .expect("transition");
    assert!(matches!(transition, SettingsTransition::Applied(applied) if applied == target));

    assert_eq!(boundaries, [target.generation()]);
    assert_eq!(controller.snapshot().applied(), target);
    let frame = camera
        .capture_next(100)
        .expect("matching-generation capture");
    assert_eq!(
        frame.data()[258 * zwo_asi::WIDTH + 258],
        171,
        "gain 200 applied"
    );
}

#[cfg(feature = "camera-substitute")]
#[test]
fn failed_apply_restores_only_the_previous_fully_applied_tuple() {
    use obscam::{SettingsTransition, apply_pending_settings};
    use zwo_asi::{CameraSource, DeterministicCamera, DeterministicScenario, Settings};

    let mut inner =
        DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
    inner
        .configure(Settings::new(500_000, 100).expect("defaults"))
        .expect("configure");
    inner.start().expect("start");
    let mut camera = FailNextConfiguration::new(inner);
    let controller = ready_controller(CameraSettings::default());
    controller.install_interrupter(camera.interrupter());
    let target = controller
        .accept(CameraSettings::new(20, 600, Treatment::Colour).expect("target"))
        .expect("camera ready");
    let mut boundary_count = 0;

    let transition = apply_pending_settings(&mut camera, &controller, |_| boundary_count += 1)
        .expect("previous tuple restores");

    assert!(matches!(
        transition,
        SettingsTransition::Restored { target: failed, .. } if failed == target
    ));
    assert_eq!(boundary_count, 0, "failed target establishes no boundary");
    assert_eq!(controller.snapshot().applied().generation(), 0);
    let frame = loop {
        match camera.capture_next(100) {
            Ok(frame) => break frame,
            Err(zwo_asi::CaptureError::Timeout) => {}
            Err(error) => panic!("restored capture failed: {error}"),
        }
    };
    assert_eq!(
        frame.data()[258 * zwo_asi::WIDTH + 258],
        166,
        "gain 100 restored"
    );
}

fn ready_controller(defaults: CameraSettings) -> SettingsController {
    let controller = SettingsController::new(defaults);
    controller.mark_camera_ready();
    controller
}

#[cfg(feature = "camera-substitute")]
struct FailNextConfiguration {
    inner: zwo_asi::DeterministicCamera,
    fail_next: bool,
}

#[cfg(feature = "camera-substitute")]
impl FailNextConfiguration {
    const fn new(inner: zwo_asi::DeterministicCamera) -> Self {
        Self {
            inner,
            fail_next: true,
        }
    }
}

#[cfg(feature = "camera-substitute")]
impl zwo_asi::CameraSource for FailNextConfiguration {
    fn interrupter(&self) -> zwo_asi::CaptureInterrupter {
        self.inner.interrupter()
    }

    fn configure(&mut self, settings: zwo_asi::Settings) -> Result<(), zwo_asi::CameraError> {
        if self.fail_next {
            self.fail_next = false;
            return Err(zwo_asi::CameraError::InvalidState {
                operation: "injected settings failure",
            });
        }
        self.inner.configure(settings)
    }

    fn start(&mut self) -> Result<(), zwo_asi::CameraError> {
        self.inner.start()
    }

    fn capture_next(
        &mut self,
        wait_ms: i32,
    ) -> Result<zwo_asi::FrameGeneration<'_>, zwo_asi::CaptureError> {
        self.inner.capture_next(wait_ms)
    }

    fn stop(&mut self) -> Result<(), zwo_asi::CameraError> {
        self.inner.stop()
    }
}
