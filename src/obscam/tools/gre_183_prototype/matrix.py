"""Scenario matrix construction for GRE-183 screening runs."""

from __future__ import annotations

from collections.abc import Iterable

from obscam.tools.gre_183_prototype.contract import (
    AcquisitionMode,
    CameraCapabilities,
    ImageFormat,
    Scenario,
)

DEFAULT_EXPOSURES_MS = (10.0, 25.0, 50.0, 100.0, 200.0, 1000.0)
DEFAULT_GAIN = 250
DEFAULT_WARMUP_FRAMES = 5


def build_mode_overlap_matrix(
    *,
    exposures_ms: Iterable[float],
    image_format: ImageFormat = ImageFormat.RAW8,
    gain: int = 0,
    high_speed: int = 0,
    bandwidth: int = 50,
    modes: Iterable[AcquisitionMode] = tuple(AcquisitionMode),
) -> list[Scenario]:
    """Build matched video/snapshot cells without assuming a mode crossover."""
    scenarios: list[Scenario] = []
    for exposure_ms in exposures_ms:
        exposure_us = int(exposure_ms * 1000)
        duration_s = max(5.0, (exposure_ms / 1000) * 3.2)
        timeout_ms = max(500, int((exposure_ms * 2) + 500))
        for acquisition_mode in modes:
            scenarios.append(
                Scenario(
                    image_format=image_format,
                    exposure_us=exposure_us,
                    gain=gain,
                    high_speed=high_speed,
                    bandwidth=bandwidth,
                    duration_s=duration_s,
                    warmup_frames=1,
                    timeout_ms=timeout_ms,
                    acquisition_mode=acquisition_mode,
                )
            )
    return scenarios


def _control_values(minimum: int, default: int, maximum: int) -> list[int]:
    return list(dict.fromkeys((minimum, default, maximum)))


def build_screening_matrix(
    capabilities: CameraCapabilities,
    *,
    duration_s: float = 60.0,
    exposures_ms: Iterable[float] = DEFAULT_EXPOSURES_MS,
    gain: int = DEFAULT_GAIN,
) -> list[Scenario]:
    """Build baseline and fast-exposure control-sweep scenarios."""
    formats = [
        image_format
        for image_format in (ImageFormat.Y8, ImageFormat.RAW8, ImageFormat.RGB24)
        if image_format in capabilities.supported_formats
    ]
    if not formats:
        raise ValueError("camera exposes none of the required GRE-183 formats")

    high_speed_default = (
        capabilities.high_speed.default if capabilities.high_speed is not None else 0
    )
    scenarios: list[Scenario] = []
    seen: set[tuple[ImageFormat, int, int, int]] = set()

    def add(
        image_format: ImageFormat,
        exposure_us: int,
        high_speed: int,
        bandwidth: int,
    ) -> None:
        key = (image_format, exposure_us, high_speed, bandwidth)
        if key in seen:
            return
        seen.add(key)
        scenarios.append(
            Scenario(
                image_format=image_format,
                exposure_us=exposure_us,
                gain=gain,
                high_speed=high_speed,
                bandwidth=bandwidth,
                duration_s=duration_s,
                warmup_frames=DEFAULT_WARMUP_FRAMES,
                timeout_ms=max(500, int((exposure_us / 1000) * 2 + 500)),
            )
        )

    exposure_values_us = [int(exposure_ms * 1000) for exposure_ms in exposures_ms]
    for image_format in formats:
        for exposure_us in exposure_values_us:
            add(
                image_format,
                exposure_us,
                high_speed_default,
                capabilities.bandwidth.default,
            )

    shortest_exposure_us = min(exposure_values_us)
    bandwidth_values = _control_values(
        capabilities.bandwidth.minimum,
        capabilities.bandwidth.default,
        capabilities.bandwidth.maximum,
    )
    high_speed_values = [high_speed_default]
    if capabilities.high_speed is not None and capabilities.high_speed.writable:
        high_speed_values = _control_values(
            capabilities.high_speed.minimum,
            capabilities.high_speed.default,
            capabilities.high_speed.maximum,
        )
    for image_format in formats:
        for high_speed in high_speed_values:
            for bandwidth in bandwidth_values:
                add(image_format, shortest_exposure_us, high_speed, bandwidth)

    return scenarios
