use obscam::ColourProcessor;
use zwo_asi::{CameraSource, DeterministicCamera, DeterministicScenario, HEIGHT, Settings, WIDTH};

const Y_BYTES: usize = WIDTH * HEIGHT;
const CHROMA_BYTES: usize = Y_BYTES / 4;

#[test]
fn deterministic_rggb_becomes_full_resolution_neutral_colour_i420() {
    let mut camera =
        DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
    camera
        .configure(Settings::new(50_000, 0).expect("settings"))
        .expect("configure");
    camera.start().expect("start");
    let source = camera.capture_next(100).expect("generation");

    let mut processor = ColourProcessor::new();
    let output = processor.process(&source);

    assert_eq!((output.width(), output.height()), (1920, 1080));
    assert_eq!(output.data().len(), Y_BYTES + CHROMA_BYTES * 2);
    let chroma_index = (258 / 2) * (WIDTH / 2) + 258 / 2;
    assert_eq!(output.data()[258 * WIDTH + 258], 109, "BT.601 Y");
    assert_eq!(output.data()[Y_BYTES + chroma_index], 85, "BT.601 U");
    assert_eq!(
        output.data()[Y_BYTES + CHROMA_BYTES + chroma_index],
        165,
        "BT.601 V"
    );
}

#[test]
fn colour_processing_reuses_one_bounded_output_buffer() {
    let mut camera =
        DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
    camera
        .configure(Settings::new(50_000, 0).expect("settings"))
        .expect("configure");
    camera.start().expect("start");
    let mut processor = ColourProcessor::new();

    let first = camera.capture_next(100).expect("first generation");
    let address = processor.process(&first).data().as_ptr();
    let second = camera.capture_next(100).expect("second generation");
    assert_eq!(processor.process(&second).data().as_ptr(), address);
}
