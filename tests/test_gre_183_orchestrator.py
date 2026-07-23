"""Unit coverage for GRE-183 process monitoring and command construction."""

import sys
from pathlib import Path

import pytest

from obscam.tools.gre_183_prototype.contract import (
    ImageFormat,
    RunnerKind,
    Scenario,
)
from obscam.tools.gre_183_prototype.orchestrator import (
    ResourceSample,
    artifact_path,
    is_recoverable_camera_error,
    parse_process_stat,
    parse_process_status,
    runner_command,
    summarize_resources,
)


def make_scenario() -> Scenario:
    """Return a compact smoke scenario."""
    return Scenario(
        image_format=ImageFormat.Y8,
        exposure_us=10_000,
        gain=250,
        high_speed=1,
        bandwidth=80,
        duration_s=2.0,
        warmup_frames=1,
        timeout_ms=520,
    )


def test_parse_process_stat_handles_spaces_in_command_name() -> None:
    prefix = "123 (runner with spaces) S"
    fields_after_state = ["0"] * 20
    fields_after_state[10] = "250"
    fields_after_state[11] = "50"
    stat_text = f"{prefix} {' '.join(fields_after_state)}"

    user_cpu_s, system_cpu_s = parse_process_stat(stat_text, 100)

    assert user_cpu_s == 2.5
    assert system_cpu_s == 0.5


def test_parse_process_status_converts_kibibytes_to_bytes() -> None:
    rss_bytes, high_water_bytes = parse_process_status(
        "Name:\trunner\nVmHWM:\t2048 kB\nVmRSS:\t1024 kB\n"
    )

    assert rss_bytes == 1024 * 1024
    assert high_water_bytes == 2048 * 1024


def test_runner_command_uses_identical_scenario_arguments() -> None:
    scenario = make_scenario()
    build_directory = Path("/tmp/gre-183-test")

    python_command = runner_command(
        RunnerKind.PYTHON_ZWOASI,
        scenario,
        build_directory=build_directory,
    )
    c_command = runner_command(
        RunnerKind.NATIVE_C,
        scenario,
        build_directory=build_directory,
    )

    assert python_command[:3] == [
        sys.executable,
        "-m",
        "obscam.tools.gre_183_prototype.python_runner",
    ]
    assert c_command[0] == "/tmp/gre-183-test/gre-183-native-c"
    assert python_command[3:] == c_command[1:]


def test_artifact_path_is_stable_for_a_runner_scenario_pair() -> None:
    output_path = artifact_path(
        RunnerKind.RUST_PIPELINE,
        make_scenario(),
        Path("results"),
    )

    assert output_path == Path("results/rust-pipeline-y8-10000us-hs1-bw80.json")


@pytest.mark.parametrize(
    "error",
    [
        "required camera ASI662MC not found",
        "Required camera ASI662MC could not be opened",
        "capture failed with ASI error 5",
    ],
)
def test_camera_start_errors_are_recoverable(error: str) -> None:
    assert is_recoverable_camera_error(error)


def test_invalid_runner_output_is_not_a_recoverable_camera_error() -> None:
    assert not is_recoverable_camera_error("runner emitted invalid result")


def test_summarize_resources_reports_observed_peaks() -> None:
    samples = [
        ResourceSample(
            elapsed_s=0.0,
            user_cpu_s=1.0,
            system_cpu_s=0.5,
            rss_bytes=100,
            high_water_rss_bytes=120,
            temperature_c=45.0,
            system_load_1m=0.5,
        ),
        ResourceSample(
            elapsed_s=1.0,
            user_cpu_s=1.6,
            system_cpu_s=0.7,
            rss_bytes=110,
            high_water_rss_bytes=140,
            temperature_c=47.0,
            system_load_1m=0.6,
        ),
    ]

    summary = summarize_resources(samples)

    assert summary.peak_rss_bytes == 110
    assert summary.peak_high_water_rss_bytes == 140
    assert summary.peak_temperature_c == 47.0
    assert summary.observed_cpu_s == pytest.approx(0.8)
