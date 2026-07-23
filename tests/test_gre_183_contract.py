"""Unit coverage for the GRE-183 benchmark contract."""

import pytest
from pydantic import ValidationError

from obscam.tools.gre_183_prototype.contract import (
    CaptureResult,
    ImageFormat,
    RunnerKind,
    Scenario,
    TimingSummary,
)


def make_scenario() -> Scenario:
    """Return a representative full-frame Y8 scenario."""
    return Scenario(
        image_format=ImageFormat.Y8,
        exposure_us=10_000,
        gain=250,
        high_speed=1,
        bandwidth=80,
        duration_s=60.0,
        timeout_ms=520,
    )


def empty_timings() -> TimingSummary:
    """Return an empty timing distribution."""
    return TimingSummary(
        count=0,
        minimum_ms=None,
        mean_ms=None,
        p50_ms=None,
        p95_ms=None,
        p99_ms=None,
        maximum_ms=None,
    )


def test_scenario_reports_exact_buffer_size_and_runner_arguments() -> None:
    scenario = make_scenario()

    assert scenario.buffer_bytes == 1920 * 1080
    assert scenario.runner_arguments() == [
        "--format",
        "Y8",
        "--exposure-us",
        "10000",
        "--gain",
        "250",
        "--high-speed",
        "1",
        "--bandwidth",
        "80",
        "--duration-s",
        "60.0",
        "--warmup-frames",
        "5",
        "--timeout-ms",
        "520",
    ]


def test_rgb24_scenario_accounts_for_three_bytes_per_pixel() -> None:
    scenario = make_scenario().model_copy(update={"image_format": ImageFormat.RGB24})

    assert scenario.buffer_bytes == 1920 * 1080 * 3


def test_capture_result_rejects_inconsistent_drop_counter() -> None:
    with pytest.raises(ValidationError, match="sdk_dropped_delta"):
        CaptureResult(
            runner=RunnerKind.PYTHON_ZWOASI,
            scenario=make_scenario(),
            sdk_version="1, 38, 0, 0",
            camera_name="ASI662MC",
            elapsed_s=1.0,
            frames=1,
            unique_frames=1,
            adjacent_duplicates=0,
            cadence_fps=1.0,
            unique_cadence_fps=1.0,
            sdk_dropped_start=2,
            sdk_dropped_end=3,
            sdk_dropped_delta=0,
            capture_errors=0,
            corrupt_frames=0,
            pipeline_drops=0,
            buffer_allocations=1,
            buffer_allocation_bytes=1920 * 1080,
            downstream_copy_bytes=0,
            capture_call_ms=empty_timings(),
            inter_frame_ms=empty_timings(),
            unique_inter_frame_ms=empty_timings(),
        )
