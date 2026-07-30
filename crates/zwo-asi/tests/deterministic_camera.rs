#![cfg(feature = "camera-substitute")]

use zwo_asi::{
    BayerLayout, CameraError, CameraSource, CaptureError, CapturePlan, DeterministicCamera,
    DeterministicScenario, Settings,
};

fn capture_one(source: &mut impl CameraSource) -> Result<(u64, usize, BayerLayout), CaptureError> {
    let frame = source.capture_next(100)?;
    Ok((frame.generation(), frame.data().len(), frame.bayer_layout()))
}

#[test]
fn scripted_faults_fail_closed_and_recovery_resumes_with_the_next_generation() {
    let absent = DeterministicScenario::new([]).camera_present(false);
    assert_eq!(
        DeterministicCamera::connect(absent).err(),
        Some(CameraError::IdentityCount { found: 0 })
    );

    let scenario = DeterministicScenario::new([
        CapturePlan::Disconnect,
        CapturePlan::MalformedDimensions {
            width: 1280,
            height: 720,
        },
        CapturePlan::MalformedLength {
            length: 1920 * 1080 - 1,
        },
        CapturePlan::Frame {
            additional_delay_us: 0,
        },
    ]);
    let mut camera = DeterministicCamera::connect(scenario).expect("camera present");
    let settings = Settings::new(10_000, 0).expect("valid settings");
    camera.configure(settings).expect("configure");
    camera.start().expect("start");

    assert_eq!(capture_one(&mut camera), Err(CaptureError::Disconnected));
    assert_eq!(capture_one(&mut camera), Err(CaptureError::NotCapturing));

    camera.recover();
    camera.configure(settings).expect("reconfigure");
    camera.start().expect("restart");
    assert_eq!(
        capture_one(&mut camera),
        Err(CaptureError::MalformedDimensions {
            width: 1280,
            height: 720,
        })
    );
    assert_eq!(
        capture_one(&mut camera),
        Err(CaptureError::MalformedLength {
            length: 1920 * 1080 - 1,
        })
    );
    assert_eq!(capture_one(&mut camera).map(|frame| frame.0), Ok(1));
}

#[test]
fn generated_pattern_identifies_orientation_bayer_gain_and_generation() {
    let mut camera =
        DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
    camera
        .configure(Settings::new(10_000, 0).expect("valid settings"))
        .expect("configure");
    camera.start().expect("start");

    let first = camera.capture_next(100).expect("first frame");
    let sample = |x: usize, y: usize| first.data()[y * 1920 + x];
    assert_eq!(sample(256, 256), 161, "red site");
    assert_eq!(sample(257, 256), 97, "green site on red row");
    assert_eq!(sample(256, 257), 97, "green site on blue row");
    assert_eq!(sample(257, 257), 33, "blue site");
    assert_eq!(sample(1918, 0), 185, "top-right orientation anchor");
    assert_eq!(sample(0, 1078), 178, "bottom-left orientation anchor");
    assert_eq!(sample(1918, 1078), 172, "bottom-right orientation anchor");
    assert_eq!((sample(0, 0), sample(2, 0)), (240, 16));

    camera.stop().expect("stop");
    camera
        .configure(Settings::new(10_000, 600).expect("valid settings"))
        .expect("reconfigure");
    camera.start().expect("restart");
    let second = camera.capture_next(100).expect("second frame");
    let sample = |x: usize, y: usize| second.data()[y * 1920 + x];
    assert_eq!(
        sample(256, 256),
        192,
        "gain and generation change the pattern"
    );
    assert_eq!((sample(0, 0), sample(2, 0)), (17, 239));
}

#[test]
fn every_pixel_marks_adjacent_generation_boundaries() {
    let mut camera =
        DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
    camera
        .configure(Settings::new(10_000, 0).expect("valid settings"))
        .expect("configure");
    camera.start().expect("start");

    let first = camera
        .capture_next(100)
        .expect("first frame")
        .data()
        .to_vec();
    let second = camera.capture_next(100).expect("second frame");

    assert!(
        first
            .iter()
            .zip(second.data())
            .all(|(first, second)| first != second),
        "a generation splice must be observable anywhere in the frame"
    );
}

#[test]
fn virtual_exposure_interruption_and_timeouts_never_publish_stale_generations() {
    let scenario = DeterministicScenario::new([CapturePlan::Frame {
        additional_delay_us: 240_000,
    }]);
    let mut camera = DeterministicCamera::connect(scenario).expect("camera present");
    camera
        .configure(Settings::new(10_000, 100).expect("complete settings"))
        .expect("configure");
    camera.start().expect("start");

    assert_eq!(capture_one(&mut camera), Err(CaptureError::Timeout));
    assert_eq!(capture_one(&mut camera), Err(CaptureError::Timeout));
    camera.interrupter().interrupt();
    assert_eq!(capture_one(&mut camera), Err(CaptureError::Interrupted));
    assert_eq!(capture_one(&mut camera).map(|frame| frame.0), Ok(1));

    assert_eq!(
        camera.configure(Settings::new(20_000, 200).expect("complete settings")),
        Err(CameraError::InvalidState {
            operation: "configure while capturing",
        })
    );
}

#[test]
fn deterministic_source_uses_the_production_capture_contract() {
    let scenario = DeterministicScenario::new([
        CapturePlan::Timeout,
        CapturePlan::Frame {
            additional_delay_us: 0,
        },
    ]);
    let mut camera = DeterministicCamera::connect(scenario).expect("camera present");
    camera
        .configure(Settings::new(10_000, 0).expect("valid settings"))
        .expect("configure");
    camera.start().expect("start");

    assert_eq!(capture_one(&mut camera), Err(CaptureError::Timeout));
    assert_eq!(
        capture_one(&mut camera),
        Ok((1, 1920 * 1080, BayerLayout::Rggb))
    );
}
