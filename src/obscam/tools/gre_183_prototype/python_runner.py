"""Command-line entry point for the corrected python-zwoasi runner."""

from __future__ import annotations

import argparse

from obscam.tools.gre_183_prototype.contract import ImageFormat, Scenario
from obscam.tools.gre_183_prototype.python_backend import (
    DEFAULT_SDK_LIBRARY,
    REQUIRED_CAMERA_MODEL,
    PythonZwoasiBackend,
)
from obscam.tools.gre_183_prototype.runner_core import run_capture_benchmark


def parse_args() -> argparse.Namespace:
    """Parse the common GRE-183 runner arguments."""
    parser = argparse.ArgumentParser(
        description="GRE-183 corrected python-zwoasi fixed-buffer runner"
    )
    parser.add_argument("--format", type=ImageFormat, required=True)
    parser.add_argument("--exposure-us", type=int, required=True)
    parser.add_argument("--gain", type=int, required=True)
    parser.add_argument("--high-speed", type=int, choices=(0, 1), required=True)
    parser.add_argument("--bandwidth", type=int, required=True)
    parser.add_argument("--duration-s", type=float, required=True)
    parser.add_argument("--warmup-frames", type=int, default=5)
    parser.add_argument("--timeout-ms", type=int, required=True)
    parser.add_argument("--sdk-library", default=DEFAULT_SDK_LIBRARY)
    parser.add_argument("--camera-model", default=REQUIRED_CAMERA_MODEL)
    return parser.parse_args()


def main() -> None:
    """Run one scenario and emit one validated JSON result."""
    args = parse_args()
    scenario = Scenario(
        image_format=args.format,
        exposure_us=args.exposure_us,
        gain=args.gain,
        high_speed=args.high_speed,
        bandwidth=args.bandwidth,
        duration_s=args.duration_s,
        warmup_frames=args.warmup_frames,
        timeout_ms=args.timeout_ms,
    )
    backend = PythonZwoasiBackend(
        library_path=args.sdk_library,
        required_model=args.camera_model,
    )
    try:
        result = run_capture_benchmark(backend, scenario)
    finally:
        backend.close()
    print(result.model_dump_json())


if __name__ == "__main__":
    main()
