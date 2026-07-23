"""Unit coverage for the fixed-buffer benchmark loop."""

from collections.abc import Sequence

from obscam.tools.gre_183_prototype.contract import (
    ImageFormat,
    RunnerKind,
    Scenario,
)
from obscam.tools.gre_183_prototype.runner_core import run_capture_benchmark


class FakeClock:
    """Deterministic monotonic clock advanced by the fake backend."""

    def __init__(self) -> None:
        self.current_ns = 0

    def now_ns(self) -> int:
        """Return the current fake time."""
        return self.current_ns

    def advance_ms(self, milliseconds: int) -> None:
        """Advance fake time by the requested milliseconds."""
        self.current_ns += milliseconds * 1_000_000


class FakeBackend:
    """Predictable caller-owned-buffer capture backend."""

    runner_kind = RunnerKind.PYTHON_ZWOASI
    sdk_version = "test-sdk"
    camera_name = "ASI662MC"

    def __init__(self, clock: FakeClock, frame_values: Sequence[int]) -> None:
        self.clock = clock
        self.frame_values = iter(frame_values)
        self.started = False

    def start(self, scenario: Scenario) -> None:
        """Mark capture active."""
        self.started = True

    def capture_into(self, buffer: bytearray, timeout_ms: int) -> None:
        """Fill the supplied buffer and advance by one 10 ms frame."""
        buffer[:] = bytes([next(self.frame_values)]) * len(buffer)
        self.clock.advance_ms(10)

    def dropped_frames(self) -> int:
        """Return a stable dropped-frame counter."""
        return 2

    def stop(self) -> None:
        """Mark capture stopped."""
        self.started = False


def test_run_capture_benchmark_tracks_duplicates_and_reuses_one_buffer() -> None:
    clock = FakeClock()
    backend = FakeBackend(clock, [1, 1, 2])
    scenario = Scenario(
        image_format=ImageFormat.Y8,
        exposure_us=10_000,
        gain=250,
        high_speed=0,
        bandwidth=80,
        duration_s=0.03,
        warmup_frames=0,
        timeout_ms=520,
    )

    result = run_capture_benchmark(
        backend,
        scenario,
        monotonic_ns=clock.now_ns,
    )

    assert result.frames == 3
    assert result.unique_frames == 2
    assert result.adjacent_duplicates == 1
    assert result.cadence_fps == 100.0
    assert result.unique_cadence_fps == 2 / 0.03
    assert result.capture_call_ms.p50_ms == 10.0
    assert result.inter_frame_ms.count == 2
    assert result.unique_inter_frame_ms.p50_ms == 20.0
    assert result.buffer_allocations == 1
    assert result.buffer_allocation_bytes == scenario.buffer_bytes
    assert result.downstream_copy_bytes == 0
    assert backend.started is False
