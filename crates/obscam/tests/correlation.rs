use obscam::{CorrelationError, CorrelationTracker, FrameSubmission, Treatment};
use uuid::Uuid;

fn submission(source_generation: u64, settings_generation: u64) -> FrameSubmission {
    FrameSubmission::new(
        Uuid::from_u128(1),
        7,
        source_generation,
        settings_generation,
        Treatment::Monochrome,
        1920,
        1080,
        10_000 + source_generation,
        20_000 + source_generation,
        false,
    )
}

#[test]
fn input_timeline_maps_exactly_across_rtp_wrap() {
    let mut tracker = CorrelationTracker::new(8, 4_500);
    tracker.submit(submission(41, 3));
    tracker.submit(submission(42, 3));

    let first = tracker
        .anchor(0, u32::MAX - 1_999)
        .expect("first exact observation");
    let second = tracker.observe(2_500).expect("wrapped exact observation");

    assert_eq!(first.source_generation(), 41);
    assert_eq!(second.source_generation(), 42);
    assert_eq!(second.rtp_timestamp(), 2_500);
}

#[test]
fn whole_step_skip_discards_only_the_missing_submission() {
    let mut tracker = CorrelationTracker::new(8, 4_500);
    for generation in 1..=3 {
        tracker.submit(submission(generation, 0));
    }
    tracker.anchor(0, 90_000).expect("anchor");

    let surviving = tracker.observe(99_000).expect("whole-step survivor");

    assert_eq!(surviving.source_generation(), 3);
    assert_eq!(tracker.encoder_skips(), 1);
    assert_eq!(tracker.lookup(94_500), None);
}

#[test]
fn incomplete_ambiguous_conflicting_and_evicted_evidence_is_unknown() {
    let mut tracker = CorrelationTracker::new(2, 4_500);
    tracker.submit(submission(1, 0));
    tracker.submit(submission(2, 0));
    tracker.submit(submission(3, 0));
    assert_eq!(tracker.anchor(0, 1), Err(CorrelationError::Evicted));

    let mut tracker = CorrelationTracker::new(4, 4_500);
    tracker.submit(submission(1, 0));
    tracker.submit(submission(2, 0));
    tracker.anchor(0, 10_000).expect("anchor");
    assert_eq!(
        tracker.observe(10_001),
        Err(CorrelationError::FractionalGap)
    );
    assert_eq!(
        tracker.observe(0x8000_0000_u32.wrapping_add(10_000)),
        Err(CorrelationError::AmbiguousGap)
    );
    assert_eq!(tracker.anchor(1, 10_000), Err(CorrelationError::Conflict));
    assert_eq!(
        tracker.lookup(10_000),
        None,
        "conflict poisons the timestamp"
    );
}

#[test]
fn repeat_preserves_the_completed_source_identity() {
    let mut tracker = CorrelationTracker::new(4, 4_500);
    tracker.submit(submission(9, 2));
    tracker.submit(submission(9, 2).as_repeat());

    tracker.anchor(0, 55_000).expect("source frame");
    let repeat = tracker.observe(59_500).expect("repeat frame");

    assert_eq!(repeat.source_generation(), 9);
    assert!(repeat.is_repeat());
}
