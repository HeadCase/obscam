"""Benchmark video-mode stream profiles across ROI and JPEG tradeoffs."""

from __future__ import annotations

import argparse
import io
import json
import statistics
import time
import zlib
from collections.abc import Iterable
from dataclasses import asdict, dataclass
from typing import Any

import zwoasi as asi  # pyright: ignore[reportMissingTypeStubs]
from PIL import Image

from obscam.camera.zwo_asi_camera import ZwoAsiCamera

DEFAULT_EXPOSURES_MS = (10.0, 25.0, 50.0, 150.0)
DEFAULT_GAIN = 300
DEFAULT_SAMPLES = 6
DEFAULT_JPEG_QUALITIES = (85, 70, 50)
SIGNATURE_SAMPLE_SIZE = 2048


@dataclass(frozen=True, slots=True)
class StreamProfileScenario:
    """One stream-profile scenario for the benchmark."""

    name: str
    jpeg_quality: int
    bins: int
    width: int
    height: int


@dataclass(slots=True)
class EncodedFrameSample:
    """One encoded-frame sample for a stream-profile scenario."""

    frame_index: int
    capture_ms: float
    encode_ms: float
    total_ms: float
    jpeg_bytes: int
    mean: float
    signature: int
    width: int
    height: int


def parse_args() -> argparse.Namespace:
    """Parse command-line arguments for the stream-profile benchmark."""
    parser = argparse.ArgumentParser(
        description="Benchmark low-latency ZWO stream profiles.",
    )
    parser.add_argument(
        "--exposure-ms",
        dest="exposures_ms",
        type=float,
        nargs="+",
        default=list(DEFAULT_EXPOSURES_MS),
        help="Exposure values to test in milliseconds.",
    )
    parser.add_argument(
        "--gain",
        type=int,
        default=DEFAULT_GAIN,
        help="Gain value to apply for every benchmark run.",
    )
    parser.add_argument(
        "--samples",
        type=int,
        default=DEFAULT_SAMPLES,
        help="Frames to capture per scenario/exposure combination.",
    )
    parser.add_argument(
        "--jpeg-quality",
        dest="jpeg_qualities",
        type=int,
        nargs="+",
        default=list(DEFAULT_JPEG_QUALITIES),
        help="JPEG quality values to include for full-frame scenarios.",
    )
    return parser.parse_args()


def round_roi_dimensions(width: int, height: int) -> tuple[int, int]:
    """Round ROI dimensions to SDK-compatible multiples."""
    rounded_width = width - (width % 8)
    rounded_height = height - (height % 2)
    return rounded_width, rounded_height


def build_default_scenarios(
    *,
    max_width: int,
    max_height: int,
    supported_bins: Iterable[int],
    jpeg_qualities: Iterable[int],
) -> list[StreamProfileScenario]:
    """Build the default set of stream-profile scenarios."""
    qualities = list(jpeg_qualities)
    scenarios = [
        StreamProfileScenario(
            name=f"full_q{quality}",
            jpeg_quality=quality,
            bins=1,
            width=max_width,
            height=max_height,
        )
        for quality in qualities
    ]

    half_width, half_height = round_roi_dimensions(max_width // 2, max_height // 2)
    scenarios.append(
        StreamProfileScenario(
            name="half_roi_q85",
            jpeg_quality=85,
            bins=1,
            width=half_width,
            height=half_height,
        )
    )

    supported_bins_set = set(supported_bins)
    if 2 in supported_bins_set:
        binned_width, binned_height = round_roi_dimensions(
            max_width // 2,
            max_height // 2,
        )
        scenarios.extend(
            [
                StreamProfileScenario(
                    name="bin2_full_q85",
                    jpeg_quality=85,
                    bins=2,
                    width=binned_width,
                    height=binned_height,
                ),
                StreamProfileScenario(
                    name="bin2_full_q70",
                    jpeg_quality=70,
                    bins=2,
                    width=binned_width,
                    height=binned_height,
                ),
            ]
        )

    return scenarios


def frame_signature(img_data: Any) -> int:
    """Build a lightweight frame fingerprint from raw image data."""
    flat_view = img_data.reshape(-1)
    sample = flat_view[:SIGNATURE_SAMPLE_SIZE]
    return zlib.crc32(sample.tobytes()) ^ len(flat_view)


def summarize_samples(samples: list[EncodedFrameSample]) -> dict[str, object]:
    """Summarize timing, size, and duplicate behavior for one sample set."""
    capture_timings = [sample.capture_ms for sample in samples]
    encode_timings = [sample.encode_ms for sample in samples]
    total_timings = [sample.total_ms for sample in samples]
    jpeg_sizes = [sample.jpeg_bytes for sample in samples]
    signatures = [sample.signature for sample in samples]
    means = [sample.mean for sample in samples]
    steady_state = samples[1:] if len(samples) > 1 else []
    steady_state_total = [sample.total_ms for sample in steady_state]
    duplicate_pairs = sum(
        1
        for previous, current in zip(signatures, signatures[1:], strict=False)
        if previous == current
    )

    return {
        "samples": [asdict(sample) for sample in samples],
        "first_frame_total_ms": round(samples[0].total_ms, 2),
        "steady_state_avg_total_ms": (
            round(statistics.mean(steady_state_total), 2)
            if steady_state_total
            else None
        ),
        "steady_state_min_total_ms": (
            round(min(steady_state_total), 2) if steady_state_total else None
        ),
        "steady_state_max_total_ms": (
            round(max(steady_state_total), 2) if steady_state_total else None
        ),
        "avg_capture_ms": round(statistics.mean(capture_timings), 2),
        "avg_encode_ms": round(statistics.mean(encode_timings), 2),
        "avg_total_ms": round(statistics.mean(total_timings), 2),
        "avg_jpeg_bytes": round(statistics.mean(jpeg_sizes), 2),
        "duplicate_adjacent_pairs": duplicate_pairs,
        "mean_brightness_min": round(min(means), 2),
        "mean_brightness_max": round(max(means), 2),
    }


def encode_jpeg(img_data: Any, *, jpeg_quality: int) -> tuple[bytes, float]:
    """Encode one frame to JPEG and return bytes plus elapsed time."""
    encode_started_at = time.perf_counter()
    image = Image.fromarray(img_data, mode="L")
    buffer = io.BytesIO()
    image.save(buffer, format="JPEG", quality=jpeg_quality)
    encode_ms = (time.perf_counter() - encode_started_at) * 1000.0
    return buffer.getvalue(), round(encode_ms, 2)


def apply_roi(camera: ZwoAsiCamera, scenario: StreamProfileScenario) -> None:
    """Apply one benchmark ROI scenario to the underlying SDK camera."""
    if camera.camera is None:
        raise RuntimeError("Camera not initialized")
    camera.camera.set_roi(
        width=scenario.width,
        height=scenario.height,
        bins=scenario.bins,
        image_type=asi.ASI_IMG_Y8,
    )


def reset_full_roi(camera: ZwoAsiCamera) -> None:
    """Restore full-frame ROI on the underlying SDK camera."""
    if camera.camera is None:
        raise RuntimeError("Camera not initialized")
    camera.camera.set_roi(
        width=None,
        height=None,
        bins=1,
        image_type=asi.ASI_IMG_Y8,
    )


def capture_encoded_samples(
    camera: ZwoAsiCamera,
    *,
    scenario: StreamProfileScenario,
    exposure_ms: float,
    gain: int,
    sample_count: int,
) -> list[EncodedFrameSample]:
    """Capture encoded-frame samples for one scenario and exposure."""
    camera.update_settings(exposure_ms=exposure_ms, gain=gain)
    apply_roi(camera, scenario)
    camera._prepare_capture(exposure_ms=exposure_ms, gain=gain)
    camera._ensure_capture_mode("video")

    samples: list[EncodedFrameSample] = []
    for frame_index in range(sample_count):
        capture_started_at = time.perf_counter()
        img_data = camera._capture_video_frame(
            timeout_ms=camera._video_capture_timeout_ms(exposure_ms)
        )
        capture_ms = (time.perf_counter() - capture_started_at) * 1000.0
        if img_data is None:
            raise RuntimeError("video capture returned no data")
        jpeg_bytes, encode_ms = encode_jpeg(
            img_data,
            jpeg_quality=scenario.jpeg_quality,
        )
        total_ms = round(capture_ms + encode_ms, 2)
        height, width = img_data.shape[:2]
        samples.append(
            EncodedFrameSample(
                frame_index=frame_index,
                capture_ms=round(capture_ms, 2),
                encode_ms=encode_ms,
                total_ms=total_ms,
                jpeg_bytes=len(jpeg_bytes),
                mean=round(float(img_data.mean()), 2),
                signature=frame_signature(img_data),
                width=width,
                height=height,
            )
        )
    return samples


def benchmark_scenarios(
    camera: ZwoAsiCamera,
    *,
    scenarios: Iterable[StreamProfileScenario],
    exposures_ms: Iterable[float],
    gain: int,
    sample_count: int,
) -> dict[str, object]:
    """Benchmark each stream-profile scenario across exposure settings."""
    results: dict[str, object] = {}
    for scenario in scenarios:
        scenario_results: dict[str, object] = {
            "scenario": asdict(scenario),
            "exposures": {},
        }
        for exposure_ms in exposures_ms:
            try:
                samples = capture_encoded_samples(
                    camera,
                    scenario=scenario,
                    exposure_ms=exposure_ms,
                    gain=gain,
                    sample_count=sample_count,
                )
            except Exception as exc:
                scenario_results["exposures"][f"{exposure_ms:.1f}"] = {
                    "error": str(exc)
                }
                continue
            scenario_results["exposures"][f"{exposure_ms:.1f}"] = summarize_samples(
                samples
            )
        results[scenario.name] = scenario_results
    return results


def main() -> None:
    """Run the stream-profile benchmark and print JSON output."""
    args = parse_args()
    camera = ZwoAsiCamera()
    if not camera.connect():
        raise SystemExit("Failed to connect to ZWO camera")

    try:
        if camera.camera is None:
            raise RuntimeError("Camera not initialized")
        camera_info = camera.camera.get_camera_property()
        scenarios = build_default_scenarios(
            max_width=int(camera_info["MaxWidth"]),
            max_height=int(camera_info["MaxHeight"]),
            supported_bins=camera_info.get("SupportedBins", [1]),
            jpeg_qualities=args.jpeg_qualities,
        )
        results = {
            "camera": {
                "name": camera_info.get("Name"),
                "max_width": camera_info.get("MaxWidth"),
                "max_height": camera_info.get("MaxHeight"),
                "supported_bins": camera_info.get("SupportedBins"),
            },
            "scenarios": benchmark_scenarios(
                camera,
                scenarios=scenarios,
                exposures_ms=args.exposures_ms,
                gain=args.gain,
                sample_count=args.samples,
            ),
        }
    finally:
        try:
            reset_full_roi(camera)
        except Exception:
            pass
        camera.disconnect()

    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
