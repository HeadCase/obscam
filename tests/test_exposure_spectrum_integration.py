"""Real-time integration tests for exposure-driven service quality."""

import asyncio
import json
import os
import threading
import time
from collections.abc import AsyncGenerator
from pathlib import Path
from typing import TypedDict, cast

import pytest
from fastapi import Request
from fastapi.responses import Response

from obscam.api.routers import camera as camera_routes
from obscam.api.routers import stream as stream_routes
from obscam.api.runtime import ApiRuntimeState
from obscam.core.backend_service import CameraBackendService
from obscam.core.frame_buffer import FrameSnapshot

EXPOSURE_SWEEP_MS = [50.0, 100.0, 200.0, 300.0, 500.0, 1000.0, 2000.0, 5000.0]
INITIAL_EXPOSURE_MS = 25.0
DEFAULT_GAIN = 250
LATEST_FRAME_MAX_AGE_S = 1.0

pytestmark = [
    pytest.mark.integration,
    pytest.mark.skipif(
        os.getenv("OBSCAM_RUN_INTEGRATION") != "1",
        reason="set OBSCAM_RUN_INTEGRATION=1 to run real-time integration tests",
    ),
]


class FakeRequest:
    """Minimal request stub for route-level settings updates."""

    def __init__(self, payload: dict[str, object]) -> None:
        self.payload = payload

    async def json(self) -> dict[str, object]:
        return self.payload


class TelemetryPayload(TypedDict, total=False):
    """Typed view of the SSE telemetry payload used by this test."""

    timestamp: float | None
    capture_ms: float | None
    has_frame: bool
    fps: float | None
    settings: dict[str, object]
    settings_version: int


class TimedCamera:
    """Camera test double that captures at real wall-clock exposure durations."""

    def __init__(self, initial_exposure_ms: float = INITIAL_EXPOSURE_MS) -> None:
        self._lock = threading.Lock()
        self._settings: dict[str, float | int] = {
            "exposure_ms": initial_exposure_ms,
            "gain": DEFAULT_GAIN,
        }
        self._connected = False
        self._frame_index = 0

    def connect(self) -> bool:
        self._connected = True
        return True

    def disconnect(self) -> None:
        self._connected = False

    def get_status(self) -> dict[str, object]:
        with self._lock:
            settings = self._settings.copy()
        return {
            "status": "connected" if self._connected else "disconnected",
            "current_exposure_ms": settings["exposure_ms"],
            "current_gain": settings["gain"],
        }

    def capture_frame(self) -> bytes | None:
        if not self._connected:
            return None

        with self._lock:
            exposure_ms = float(self._settings["exposure_ms"])
            gain = int(self._settings["gain"])

        time.sleep(exposure_ms / 1000.0)

        with self._lock:
            self._frame_index += 1
            frame_index = self._frame_index

        return f"frame-{frame_index:04d}-exp-{exposure_ms:.1f}-gain-{gain}".encode(
            "ascii"
        )

    def update_settings(self, **settings: object) -> bool:
        try:
            with self._lock:
                if "exposure_ms" in settings:
                    exposure = settings["exposure_ms"]
                    assert isinstance(exposure, int | float)
                    self._settings["exposure_ms"] = float(exposure)
                if "gain" in settings:
                    gain = settings["gain"]
                    assert isinstance(gain, int | float)
                    self._settings["gain"] = int(gain)
        except Exception:
            return False
        return True

    def get_current_settings(self) -> dict[str, object]:
        with self._lock:
            return cast(dict[str, object], self._settings.copy())

    def get_control_capabilities(self) -> dict[str, object]:
        return {
            "exposure_ms": {"min": 0.032, "max": 30000.0, "type": "float"},
            "gain": {"min": 0, "max": 600, "type": "int"},
        }


def _request(payload: dict[str, object]) -> Request:
    return cast(Request, FakeRequest(payload))


def _parse_sse_payload(chunk: str) -> TelemetryPayload:
    payload = chunk.split("data:", maxsplit=1)[1].strip()
    return cast(TelemetryPayload, json.loads(payload))


async def _read_sse_payload(
    body_iterator: AsyncGenerator[str, None],
    timeout_s: float,
) -> TelemetryPayload:
    chunk = await asyncio.wait_for(anext(body_iterator), timeout=timeout_s)
    return _parse_sse_payload(chunk)


async def _wait_for_matching_snapshot(
    backend: CameraBackendService,
    target_exposure_ms: float,
    min_generation: int,
    timeout_s: float,
) -> FrameSnapshot:
    deadline = time.monotonic() + timeout_s
    while True:
        snapshot = backend.frame_buffer.get_latest_snapshot()
        if snapshot is not None and snapshot.generation > min_generation:
            exposure_ms = snapshot.metadata.get("exposure_ms")
            if (
                isinstance(exposure_ms, int | float)
                and abs(float(exposure_ms) - target_exposure_ms) < 1e-6
            ):
                return snapshot

        if time.monotonic() >= deadline:
            raise AssertionError(
                f"Timed out waiting for frame at exposure {target_exposure_ms}ms"
            )

        await asyncio.sleep(0.01)


async def _wait_for_matching_payload(
    body_iterator: AsyncGenerator[str, None],
    target_exposure_ms: float,
    min_timestamp: float,
    timeout_s: float,
    *,
    require_fps: bool = False,
) -> TelemetryPayload:
    deadline = time.monotonic() + timeout_s
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise AssertionError(
                f"Timed out waiting for telemetry at exposure {target_exposure_ms}ms"
            )

        payload = await _read_sse_payload(body_iterator, remaining)
        settings = payload.get("settings", {})
        settings_exposure = settings.get("exposure_ms")
        timestamp = payload.get("timestamp")

        if (
            isinstance(settings_exposure, int | float)
            and abs(float(settings_exposure) - target_exposure_ms) < 1e-6
            and isinstance(timestamp, int | float)
            and float(timestamp) >= min_timestamp
            and (not require_fps or isinstance(payload.get("fps"), int | float))
        ):
            return payload


def _assert_capture_duration(
    payload: TelemetryPayload,
    target_exposure_ms: float,
) -> float:
    capture_ms = payload.get("capture_ms")
    assert isinstance(capture_ms, int | float)
    tolerance_ms = max(75.0, target_exposure_ms * 0.1)
    assert abs(float(capture_ms) - target_exposure_ms) <= tolerance_ms
    return float(capture_ms)


def _assert_fps(payload: TelemetryPayload, capture_ms: float) -> None:
    fps = payload.get("fps")
    assert isinstance(fps, int | float)
    assert float(fps) > 0.0

    expected_fps = 1000.0 / capture_ms
    lower_bound = expected_fps * 0.65
    upper_bound = expected_fps * 1.35
    assert lower_bound <= float(fps) <= upper_bound


async def _get_latest_frame_response(backend: CameraBackendService) -> Response:
    return await camera_routes.get_latest_frame(backend)


def test_exposure_sweep_preserves_browser_visible_service_quality(
    tmp_path: Path,
) -> None:
    async def scenario() -> None:
        backend = CameraBackendService(TimedCamera(), tmp_path)
        runtime = ApiRuntimeState()
        runtime.backend = backend

        assert backend.start_backend() is True
        backend.frame_buffer.on_new_frame = runtime.on_new_frame

        response = await stream_routes.telemetry_sse(backend, runtime)
        body_iterator = cast(AsyncGenerator[str, None], response.body_iterator)
        initial_payload = await _read_sse_payload(body_iterator, timeout_s=1.0)
        assert "settings" in initial_payload

        initial_snapshot = backend.frame_buffer.get_latest_snapshot()
        initial_generation = initial_snapshot.generation if initial_snapshot else 0
        previous_snapshot = await _wait_for_matching_snapshot(
            backend,
            target_exposure_ms=INITIAL_EXPOSURE_MS,
            min_generation=initial_generation,
            timeout_s=1.0,
        )
        previous_generation = previous_snapshot.generation
        previous_timestamp = float(previous_snapshot.metadata["timestamp"])
        previous_frame_bytes = previous_snapshot.frame_bytes
        previous_exposure_ms = INITIAL_EXPOSURE_MS

        try:
            for exposure_ms in EXPOSURE_SWEEP_MS:
                runtime.delivery_notifier.bind(
                    asyncio.get_running_loop(),
                    frame_generation=previous_generation,
                    current_settings_version=runtime.settings_version,
                )
                runtime.delivery_notifier._frame_times.clear()
                await camera_routes.update_settings(
                    _request({"exposure_ms": exposure_ms}),
                    backend,
                    runtime,
                )

                timeout_s = (
                    (previous_exposure_ms / 1000.0) + (exposure_ms / 1000.0) + 2.5
                )
                snapshot = await _wait_for_matching_snapshot(
                    backend,
                    target_exposure_ms=exposure_ms,
                    min_generation=previous_generation,
                    timeout_s=timeout_s,
                )
                if exposure_ms <= 2000.0:
                    snapshot = await _wait_for_matching_snapshot(
                        backend,
                        target_exposure_ms=exposure_ms,
                        min_generation=snapshot.generation,
                        timeout_s=(exposure_ms / 1000.0) + 1.5,
                    )
                latest_frame = await _get_latest_frame_response(backend)
                telemetry = await _wait_for_matching_payload(
                    body_iterator,
                    target_exposure_ms=exposure_ms,
                    min_timestamp=float(snapshot.metadata["timestamp"]),
                    timeout_s=max(1.0, (exposure_ms / 1000.0) + 1.0),
                    require_fps=exposure_ms <= 2000.0,
                )

                assert latest_frame.body == snapshot.frame_bytes

                header_timestamp = float(latest_frame.headers["x-frame-timestamp"])
                assert header_timestamp == float(snapshot.metadata["timestamp"])
                assert header_timestamp > previous_timestamp

                latest_frame_age_s = float(latest_frame.headers["x-frame-age-seconds"])
                assert latest_frame_age_s <= LATEST_FRAME_MAX_AGE_S

                assert latest_frame.body != previous_frame_bytes
                assert telemetry["has_frame"] is True

                settings = telemetry["settings"]
                settings_exposure = settings.get("exposure_ms")
                assert isinstance(settings_exposure, int | float)
                assert float(settings_exposure) == exposure_ms

                capture_ms = _assert_capture_duration(telemetry, exposure_ms)
                if exposure_ms <= 2000.0:
                    _assert_fps(telemetry, capture_ms)

                previous_generation = snapshot.generation
                previous_timestamp = header_timestamp
                previous_frame_bytes = latest_frame.body
                previous_exposure_ms = exposure_ms
        finally:
            await body_iterator.aclose()
            backend.stop_backend()

    asyncio.run(scenario())
