use obscam::{LatestFrameMailbox, MonochromeProcessor};
use zwo_asi::{CameraSource, DeterministicCamera, DeterministicScenario, HEIGHT, Settings, WIDTH};

const Y_BYTES: usize = WIDTH * HEIGHT;
const I420_BYTES: usize = Y_BYTES + Y_BYTES / 2;

#[test]
fn deterministic_rggb_becomes_full_resolution_neutral_i420_without_a_bayer_grid() {
    let mut camera = deterministic_camera();
    let source = camera.capture_next(100).expect("deterministic generation");
    let mut processor = MonochromeProcessor::new();

    let output = processor.process(&source);

    assert_eq!(output.generation(), 1);
    assert_eq!((output.width(), output.height()), (1920, 1080));
    assert_eq!(output.data().len(), I420_BYTES);

    let y = &output.data()[..Y_BYTES];
    assert_eq!(
        [
            y[258 * WIDTH + 258],
            y[258 * WIDTH + 259],
            y[259 * WIDTH + 258],
            y[259 * WIDTH + 259],
        ],
        [109; 4],
        "a flat RGB patch must not retain the four-site RAW8 mosaic"
    );
    assert_eq!(y[1079 * WIDTH + 1919], 120, "bottom-right edge");
    assert_eq!(y[1079 * WIDTH], 126, "bottom-left edge");
    assert_eq!(y[1919], 133, "top-right edge");
    assert_eq!(y[0], 240, "top-left edge");
    assert!(
        output.data()[Y_BYTES..].iter().all(|sample| *sample == 128),
        "monochrome I420 chroma must be neutral"
    );
}

#[test]
fn processing_reuses_one_output_buffer_across_generations() {
    let mut camera = deterministic_camera();
    let mut processor = MonochromeProcessor::new();

    let first = camera.capture_next(100).expect("first generation");
    let first_output = processor.process(&first);
    let output_address = first_output.data().as_ptr();
    assert_eq!(first_output.generation(), 1);

    let second = camera.capture_next(100).expect("second generation");
    let second_output = processor.process(&second);
    assert_eq!(second_output.generation(), 2);
    assert_eq!(second_output.data().as_ptr(), output_address);
}

#[test]
fn publication_keeps_only_the_newest_pending_generation() {
    let mut camera = deterministic_camera();
    let mut processor = MonochromeProcessor::new();
    let mailbox = LatestFrameMailbox::new();

    for expected_generation in 1..=3 {
        let source = camera.capture_next(100).expect("source generation");
        let output = processor.process(&source);
        assert_eq!(output.generation(), expected_generation);
        mailbox.publish(&output);
    }

    let newest = mailbox.take().expect("one pending publication");
    assert_eq!(newest.generation(), 3);
    assert!(mailbox.take().is_none(), "obsolete work was replaced");
}

#[test]
fn claimed_output_remains_committable_when_newer_work_arrives() {
    let mut camera = deterministic_camera();
    let mut processor = MonochromeProcessor::new();
    let mailbox = LatestFrameMailbox::new();

    let first = camera.capture_next(100).expect("first source generation");
    mailbox.publish(&processor.process(&first));
    let executing = mailbox.take().expect("first publication starts");

    let second = camera.capture_next(100).expect("newer source generation");
    mailbox.publish(&processor.process(&second));

    assert!(
        mailbox.is_current(&executing),
        "newer same-epoch work replaces only pending work"
    );
    mailbox.recycle(executing);
    let replacement = mailbox.take().expect("newest publication remains");
    assert_eq!(replacement.generation(), 2);
}

#[test]
fn new_epoch_fences_claimed_and_pending_output() {
    let mut camera = deterministic_camera();
    let mut processor = MonochromeProcessor::new();
    let mailbox = LatestFrameMailbox::new();

    let first = camera.capture_next(100).expect("first source generation");
    mailbox.publish(&processor.process(&first));
    let claimed = mailbox.take().expect("first publication starts");

    let second = camera.capture_next(100).expect("second source generation");
    mailbox.publish(&processor.process(&second));
    mailbox.begin_new_epoch();

    assert!(!mailbox.is_current(&claimed));
    assert!(mailbox.take().is_none(), "old pending work was fenced");
}

fn deterministic_camera() -> DeterministicCamera {
    let mut camera =
        DeterministicCamera::connect(DeterministicScenario::new([])).expect("camera present");
    camera
        .configure(Settings::new(50_000, 0).expect("valid settings"))
        .expect("configure");
    camera.start().expect("start");
    camera
}
