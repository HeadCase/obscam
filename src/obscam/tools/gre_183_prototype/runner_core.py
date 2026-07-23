"""Reusable Python-side capture loop for the GRE-183 benchmark."""

from __future__ import annotations

import time
import zlib
from array import array
from collections.abc import Callable
from typing import Protocol

from obscam.tools.gre_183_prototype.contract import (
    CaptureResult,
    RunnerKind,
    Scenario,
)
from obscam.tools.gre_183_prototype.statistics import summarize_timings


class CaptureBackend(Protocol):
    """Minimal capture backend needed by the Python benchmark loop."""

    @property
    def runner_kind(self) -> RunnerKind:
        """Return the implementation identity."""

    @property
    def sdk_version(self) -> str:
        """Return the loaded SDK version."""

    @property
    def camera_name(self) -> str:
        """Return the selected camera model."""

    def start(self, scenario: Scenario) -> None:
        """Configure and start continuous acquisition."""

    def capture_into(self, buffer: bytearray, timeout_ms: int) -> None:
        """Fill a caller-owned frame buffer."""

    def dropped_frames(self) -> int:
        """Return the SDK dropped-frame counter."""

    def stop(self) -> None:
        """Stop continuous acquisition."""


def run_capture_benchmark(
    backend: CaptureBackend,
    scenario: Scenario,
    *,
    monotonic_ns: Callable[[], int] = time.perf_counter_ns,
) -> CaptureResult:
    """Run one fixed-buffer scenario and return the common result."""
    frame_buffer = bytearray(scenario.buffer_bytes)
    capture_timings = array("d")
    inter_frame_timings = array("d")
    unique_inter_frame_timings = array("d")
    frames = 0
    unique_frames = 0
    adjacent_duplicates = 0
    capture_errors = 0
    previous_crc32: int | None = None
    previous_completion_ns: int | None = None
    previous_unique_completion_ns: int | None = None

    backend.start(scenario)
    try:
        for _ in range(scenario.warmup_frames):
            backend.capture_into(frame_buffer, scenario.timeout_ms)

        sdk_dropped_start = backend.dropped_frames()
        measurement_started_ns = monotonic_ns()
        duration_ns = int(scenario.duration_s * 1_000_000_000)
        while monotonic_ns() - measurement_started_ns < duration_ns:
            capture_started_ns = monotonic_ns()
            try:
                backend.capture_into(frame_buffer, scenario.timeout_ms)
            except Exception:
                capture_errors += 1
                continue
            completion_ns = monotonic_ns()
            frames += 1
            capture_timings.append((completion_ns - capture_started_ns) / 1_000_000)
            if previous_completion_ns is not None:
                inter_frame_timings.append(
                    (completion_ns - previous_completion_ns) / 1_000_000
                )
            previous_completion_ns = completion_ns

            crc32 = zlib.crc32(frame_buffer)
            if crc32 == previous_crc32:
                adjacent_duplicates += 1
                continue
            previous_crc32 = crc32
            unique_frames += 1
            if previous_unique_completion_ns is not None:
                unique_inter_frame_timings.append(
                    (completion_ns - previous_unique_completion_ns) / 1_000_000
                )
            previous_unique_completion_ns = completion_ns

        measurement_ended_ns = monotonic_ns()
        sdk_dropped_end = backend.dropped_frames()
    finally:
        backend.stop()

    elapsed_s = (measurement_ended_ns - measurement_started_ns) / 1_000_000_000
    return CaptureResult(
        runner=backend.runner_kind,
        scenario=scenario,
        sdk_version=backend.sdk_version,
        camera_name=backend.camera_name,
        elapsed_s=elapsed_s,
        frames=frames,
        unique_frames=unique_frames,
        adjacent_duplicates=adjacent_duplicates,
        cadence_fps=frames / elapsed_s,
        unique_cadence_fps=unique_frames / elapsed_s,
        sdk_dropped_start=sdk_dropped_start,
        sdk_dropped_end=sdk_dropped_end,
        sdk_dropped_delta=sdk_dropped_end - sdk_dropped_start,
        capture_errors=capture_errors,
        corrupt_frames=0,
        pipeline_drops=0,
        buffer_allocations=1,
        buffer_allocation_bytes=len(frame_buffer),
        downstream_copy_bytes=0,
        crc32_last=previous_crc32,
        capture_call_ms=summarize_timings(capture_timings),
        inter_frame_ms=summarize_timings(inter_frame_timings),
        unique_inter_frame_ms=summarize_timings(unique_inter_frame_timings),
        notes=[
            "caller-owned SDK frame buffer reused for the complete run",
            "python-zwoasi public get_video_data creates a ctypes view per call",
            "public python-zwoasi buffer API does not expose guard regions",
        ],
    )
