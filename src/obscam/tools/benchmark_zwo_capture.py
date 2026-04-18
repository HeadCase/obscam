"""Benchmark the ZWO capture path across exposure settings and modes."""

from __future__ import annotations

import argparse
import json
import statistics
import time
import zlib
from collections.abc import Iterable
from dataclasses import asdict, dataclass
from typing import Any

from obscam.camera.zwo_asi_camera import ZwoAsiCamera

DEFAULT_EXPOSURES_MS = (10.0, 25.0, 50.0, 150.0)
DEFAULT_GAIN = 300
DEFAULT_SAMPLES = 6
DEFAULT_SIGNATURE_SAMPLE_SIZE = 2048


@dataclass(slots=True)
class FrameSample:
    """Single benchmark sample for one raw frame."""

    frame_index: int
    capture_ms: float
    mean: float
    signature: int


def parse_args() -> argparse.Namespace:
    """Parse command-line arguments for the benchmark helper."""
    parser = argparse.ArgumentParser(
        description="Benchmark obscam ZWO raw-capture behavior.",
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
        help="Frames to capture per exposure/mode combination.",
    )
    return parser.parse_args()


def frame_signature(img_data: Any) -> int:
    """Build a lightweight frame fingerprint from the raw image data."""
    flat_view = img_data.reshape(-1)
    sample = flat_view[:DEFAULT_SIGNATURE_SAMPLE_SIZE]
    return zlib.crc32(sample.tobytes()) ^ len(flat_view)


def summarize_samples(samples: list[FrameSample]) -> dict[str, object]:
    """Summarize timing and fingerprint behavior for a sample set."""
    timings = [sample.capture_ms for sample in samples]
    means = [sample.mean for sample in samples]
    signatures = [sample.signature for sample in samples]
    steady_state = samples[1:] if len(samples) > 1 else []
    steady_state_timings = [sample.capture_ms for sample in steady_state]
    duplicate_pairs = sum(
        1
        for previous, current in zip(signatures, signatures[1:], strict=False)
        if previous == current
    )
    return {
        "samples": [asdict(sample) for sample in samples],
        "first_frame_ms": round(samples[0].capture_ms, 2),
        "steady_state_avg_ms": (
            round(statistics.mean(steady_state_timings), 2)
            if steady_state_timings
            else None
        ),
        "steady_state_min_ms": (
            round(min(steady_state_timings), 2) if steady_state_timings else None
        ),
        "steady_state_max_ms": (
            round(max(steady_state_timings), 2) if steady_state_timings else None
        ),
        "all_frames_avg_ms": round(statistics.mean(timings), 2),
        "mean_brightness_min": round(min(means), 2),
        "mean_brightness_max": round(max(means), 2),
        "duplicate_adjacent_pairs": duplicate_pairs,
    }


def capture_samples(
    camera: ZwoAsiCamera,
    *,
    exposure_ms: float,
    gain: int,
    mode: str,
    sample_count: int,
) -> list[FrameSample]:
    """Capture raw-frame benchmark samples for one exposure and mode."""
    camera.update_settings(exposure_ms=exposure_ms, gain=gain)
    camera._prepare_capture(exposure_ms=exposure_ms, gain=gain)
    if mode == "video":
        switched_mode = camera._ensure_capture_mode("video")
    else:
        switched_mode = camera._ensure_capture_mode("single")

    samples: list[FrameSample] = []
    for frame_index in range(sample_count):
        capture_started_at = time.perf_counter()
        if mode == "video":
            img_data = camera._capture_video_frame(
                timeout_ms=camera._video_capture_timeout_ms(exposure_ms)
            )
        else:
            img_data = camera._capture_single_frame()
        capture_ms = (time.perf_counter() - capture_started_at) * 1000.0
        if img_data is None:
            raise RuntimeError(f"{mode} capture returned no data")
        samples.append(
            FrameSample(
                frame_index=frame_index + (1 if switched_mode else 0),
                capture_ms=round(capture_ms, 2),
                mean=round(float(img_data.mean()), 2),
                signature=frame_signature(img_data),
            )
        )
    return samples


def benchmark_modes(
    camera: ZwoAsiCamera,
    *,
    exposures_ms: Iterable[float],
    gain: int,
    sample_count: int,
) -> dict[str, object]:
    """Run the raw-frame benchmark across single and video capture modes."""
    results: dict[str, object] = {"single": {}, "video": {}}
    for mode in ("single", "video"):
        mode_results = {}
        for exposure_ms in exposures_ms:
            try:
                samples = capture_samples(
                    camera,
                    exposure_ms=exposure_ms,
                    gain=gain,
                    mode=mode,
                    sample_count=sample_count,
                )
            except Exception as exc:
                mode_results[f"{exposure_ms:.1f}"] = {"error": str(exc)}
                continue
            mode_results[f"{exposure_ms:.1f}"] = summarize_samples(samples)
        results[mode] = mode_results
    return results


def main() -> None:
    """Run the benchmark and print JSON output."""
    args = parse_args()
    camera = ZwoAsiCamera()
    if not camera.connect():
        raise SystemExit("Failed to connect to ZWO camera")

    try:
        results = benchmark_modes(
            camera,
            exposures_ms=args.exposures_ms,
            gain=args.gain,
            sample_count=args.samples,
        )
    finally:
        camera.disconnect()

    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
