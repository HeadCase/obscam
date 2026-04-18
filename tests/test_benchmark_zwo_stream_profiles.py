from obscam.tools.benchmark_zwo_stream_profiles import (
    EncodedFrameSample,
    build_default_scenarios,
    round_roi_dimensions,
    summarize_samples,
)


def test_round_roi_dimensions_uses_sdk_compatible_multiples() -> None:
    assert round_roi_dimensions(961, 541) == (960, 540)


def test_build_default_scenarios_includes_bin2_when_supported() -> None:
    scenarios = build_default_scenarios(
        max_width=1920,
        max_height=1080,
        supported_bins=[1, 2],
        jpeg_qualities=[85, 70],
    )

    scenario_names = {scenario.name for scenario in scenarios}
    assert "full_q85" in scenario_names
    assert "full_q70" in scenario_names
    assert "half_roi_q85" in scenario_names
    assert "bin2_full_q85" in scenario_names
    assert "bin2_full_q70" in scenario_names


def test_build_default_scenarios_skips_bin2_when_unsupported() -> None:
    scenarios = build_default_scenarios(
        max_width=1920,
        max_height=1080,
        supported_bins=[1],
        jpeg_qualities=[85],
    )

    scenario_names = {scenario.name for scenario in scenarios}
    assert "full_q85" in scenario_names
    assert "half_roi_q85" in scenario_names
    assert "bin2_full_q85" not in scenario_names


def test_summarize_samples_reports_steady_state_and_duplicates() -> None:
    samples = [
        EncodedFrameSample(
            frame_index=0,
            capture_ms=200.0,
            encode_ms=20.0,
            total_ms=220.0,
            jpeg_bytes=1000,
            mean=10.0,
            signature=1,
            width=1920,
            height=1080,
        ),
        EncodedFrameSample(
            frame_index=1,
            capture_ms=100.0,
            encode_ms=10.0,
            total_ms=110.0,
            jpeg_bytes=900,
            mean=11.0,
            signature=1,
            width=1920,
            height=1080,
        ),
        EncodedFrameSample(
            frame_index=2,
            capture_ms=95.0,
            encode_ms=9.0,
            total_ms=104.0,
            jpeg_bytes=880,
            mean=12.0,
            signature=2,
            width=1920,
            height=1080,
        ),
    ]

    summary = summarize_samples(samples)

    assert summary["first_frame_total_ms"] == 220.0
    assert summary["steady_state_avg_total_ms"] == 107.0
    assert summary["avg_capture_ms"] == 131.67
    assert summary["avg_encode_ms"] == 13.0
    assert summary["duplicate_adjacent_pairs"] == 1
