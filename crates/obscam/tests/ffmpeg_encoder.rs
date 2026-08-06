#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use obscam::{FfmpegEncoder, LatestFrameMailbox, MonochromeProcessor};
use uuid::Uuid;
use zwo_asi::{CameraSource, DeterministicCamera, DeterministicScenario, Settings};

#[test]
fn one_ffmpeg_child_receives_native_i420_with_the_qualified_hardware_profile() {
    let directory = std::env::temp_dir().join(format!("obscam-ffmpeg-test-{}", Uuid::new_v4()));
    fs::create_dir(&directory).expect("create test directory");
    let program = directory.join("fake-ffmpeg");
    let arguments = directory.join("arguments");
    let input = directory.join("input");
    fs::write(
        &program,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat > '{}'\n",
            arguments.display(),
            input.display()
        ),
    )
    .expect("write fake FFmpeg");
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).expect("make executable");

    let mut camera =
        DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
    camera
        .configure(Settings::new(50_000, 0).expect("settings"))
        .expect("configure");
    camera.start().expect("start");
    let source = camera.capture_next(100).expect("source generation");
    let mut processor = MonochromeProcessor::new();
    let mailbox = LatestFrameMailbox::new();
    mailbox.publish(&processor.process(&source));
    let frame = mailbox.take().expect("processed generation");

    let mut encoder = FfmpegEncoder::start(&program).expect("start fake FFmpeg");
    encoder.publish(&frame).expect("publish one complete frame");
    encoder.finish().expect("finish fake FFmpeg");

    let arguments = fs::read_to_string(arguments).expect("captured FFmpeg arguments");
    let arguments = arguments.lines().collect::<Vec<_>>();
    assert!(has_pair(&arguments, "-c:v", "h264_v4l2m2m"));
    assert!(has_pair(&arguments, "-b:v", "1500k"));
    assert!(has_pair(&arguments, "-pixel_format", "yuv420p"));
    assert!(has_pair(&arguments, "-video_size", "1920x1080"));
    assert!(has_pair(&arguments, "-framerate", "20"));
    assert!(has_pair(&arguments, "-g", "20"));
    assert!(has_pair(&arguments, "-f", "rtp"));
    assert!(has_pair(&arguments, "-payload_type", "96"));
    assert!(has_pair(&arguments, "-seq", "1000"));
    assert!(arguments.contains(&"rtp://127.0.0.1:5002?rtcpport=5003&pkt_size=1200"));
    assert!(!arguments.contains(&"libx264"));
    assert!(!arguments.contains(&"mjpeg"));
    assert_eq!(
        fs::metadata(input).expect("captured I420 input").len(),
        1920 * 1080 * 3 / 2
    );

    fs::remove_dir_all(directory).expect("remove test directory");
}

fn has_pair(arguments: &[&str], option: &str, value: &str) -> bool {
    arguments.windows(2).any(|pair| pair == [option, value])
}

#[test]
fn missing_ffmpeg_program_fails_without_an_alternate_encoder() {
    let error = FfmpegEncoder::start(Path::new("/definitely/not/ffmpeg"))
        .expect_err("missing qualified program must fail");

    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn completed_frame_remains_repeatable_across_a_settings_epoch() {
    let directory = std::env::temp_dir().join(format!("obscam-repeat-test-{}", Uuid::new_v4()));
    fs::create_dir(&directory).expect("create test directory");
    let program = directory.join("fake-ffmpeg");
    let input = directory.join("input");
    fs::write(
        &program,
        format!("#!/bin/sh\ncat > '{}'\n", input.display()),
    )
    .expect("write fake FFmpeg");
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).expect("make executable");

    let mut camera =
        DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
    camera
        .configure(Settings::new(50_000, 0).expect("settings"))
        .expect("configure");
    camera.start().expect("start");
    let source = camera.capture_next(100).expect("source generation");
    let mut processor = MonochromeProcessor::new();
    let mailbox = LatestFrameMailbox::new();
    mailbox.publish(&processor.process(&source));
    let frame = mailbox.take().expect("processed generation");

    let mut encoder = FfmpegEncoder::start(&program).expect("start fake FFmpeg");
    encoder.publish(&frame).expect("publish completed frame");
    mailbox.begin_new_epoch();
    encoder
        .publish(&frame)
        .expect("repeat last completed frame after settings boundary");
    encoder.finish().expect("finish fake FFmpeg");

    assert_eq!(
        fs::metadata(input).expect("captured I420 input").len(),
        2 * 1920 * 1080 * 3 / 2
    );
    fs::remove_dir_all(directory).expect("remove test directory");
}
