"""Unit coverage for the GRE-183 screening matrix."""

from obscam.tools.gre_183_prototype.contract import (
    AcquisitionMode,
    CameraCapabilities,
    ControlCapability,
    ImageFormat,
)
from obscam.tools.gre_183_prototype.matrix import (
    build_mode_overlap_matrix,
    build_screening_matrix,
)


def make_capabilities() -> CameraCapabilities:
    """Return representative ASI662MC capabilities."""
    return CameraCapabilities(
        camera_name="ASI662MC",
        sdk_version="1, 38, 0, 0",
        width=1920,
        height=1080,
        is_usb3_host=True,
        is_usb3_camera=True,
        supported_formats=[ImageFormat.Y8, ImageFormat.RAW8, ImageFormat.RGB24],
        bandwidth=ControlCapability(
            minimum=40,
            maximum=100,
            default=80,
            writable=True,
        ),
        high_speed=ControlCapability(
            minimum=0,
            maximum=1,
            default=0,
            writable=True,
        ),
    )


def test_screening_matrix_has_baselines_and_fast_control_sweep() -> None:
    matrix = build_screening_matrix(
        make_capabilities(),
        duration_s=2.0,
        exposures_ms=[10.0, 25.0],
    )

    keys = {
        (
            scenario.image_format,
            scenario.exposure_us,
            scenario.high_speed,
            scenario.bandwidth,
        )
        for scenario in matrix
    }
    assert (ImageFormat.Y8, 25_000, 0, 80) in keys
    assert (ImageFormat.RAW8, 10_000, 1, 40) in keys
    assert (ImageFormat.RGB24, 10_000, 1, 100) in keys
    assert len(keys) == len(matrix)


def test_screening_matrix_skips_unsupported_rgb24() -> None:
    capabilities = make_capabilities().model_copy(
        update={"supported_formats": [ImageFormat.Y8, ImageFormat.RAW8]}
    )

    matrix = build_screening_matrix(
        capabilities,
        duration_s=2.0,
        exposures_ms=[10.0],
    )

    assert all(scenario.image_format is not ImageFormat.RGB24 for scenario in matrix)


def test_mode_overlap_matrix_pairs_modes_without_a_crossover_assumption() -> None:
    scenarios = build_mode_overlap_matrix(exposures_ms=[10.0, 1000.0])

    assert [scenario.acquisition_mode for scenario in scenarios] == [
        AcquisitionMode.VIDEO,
        AcquisitionMode.SNAPSHOT,
        AcquisitionMode.VIDEO,
        AcquisitionMode.SNAPSHOT,
    ]
    assert [scenario.duration_s for scenario in scenarios] == [5.0, 5.0, 5.0, 5.0]
    assert all(scenario.warmup_frames == 1 for scenario in scenarios)
