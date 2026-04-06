"""FastAPI runtime state and streaming coordination."""

import asyncio
import time
from collections import deque
from typing import Any, cast

from fastapi import FastAPI

from obscam.core.backend_service import CameraBackendService
from obscam.core.camera_factory import get_backend_service
from obscam.core.frame_buffer import FrameSnapshot


class FrameDeliveryNotifier:
    """Coordinate loop-safe wakeups for frame and settings updates."""

    def __init__(self) -> None:
        self._loop: asyncio.AbstractEventLoop | None = None
        self._condition: asyncio.Condition | None = None
        self._frame_generation = 0
        self._settings_version = 0
        self._frame_times: deque[float] = deque(maxlen=120)

    def bind(
        self,
        loop: asyncio.AbstractEventLoop,
        frame_generation: int,
        current_settings_version: int,
    ) -> None:
        """Bind the notifier to the active event loop."""
        if self._loop is loop and self._condition is not None:
            self._frame_generation = max(self._frame_generation, frame_generation)
            self._settings_version = max(
                self._settings_version, current_settings_version
            )
            return

        self._loop = loop
        self._condition = asyncio.Condition()
        self._frame_generation = frame_generation
        self._settings_version = current_settings_version
        self._frame_times.clear()

    def publish_frame(self, snapshot: FrameSnapshot) -> None:
        """Schedule a loop-safe frame update."""
        loop = self._get_active_loop()
        if loop is None:
            return

        published_at = time.time()
        loop.call_soon_threadsafe(
            self._schedule_frame_update, snapshot.generation, published_at
        )

    def publish_settings(self, current_settings_version: int) -> None:
        """Schedule a loop-safe settings update."""
        loop = self._get_active_loop()
        if loop is None:
            return

        loop.call_soon_threadsafe(
            self._schedule_settings_update, current_settings_version
        )

    async def wait_for_frame(self, last_generation: int) -> int:
        """Wait for a newer frame generation."""
        condition = self._require_condition()
        async with condition:
            await condition.wait_for(lambda: self._frame_generation > last_generation)
            return self._frame_generation

    async def wait_for_update(
        self, last_generation: int, last_settings_version: int
    ) -> tuple[int, int]:
        """Wait for either a newer frame or a newer settings version."""
        condition = self._require_condition()
        async with condition:
            await condition.wait_for(
                lambda: (
                    self._frame_generation > last_generation
                    or self._settings_version > last_settings_version
                )
            )
            return self._frame_generation, self._settings_version

    def estimate_fps(self, window_s: float = 5.0) -> float | None:
        """Estimate FPS from recent frame arrival times."""
        if len(self._frame_times) < 2:
            return None

        now = time.time()
        recent = [
            frame_time
            for frame_time in self._frame_times
            if now - frame_time <= window_s
        ]
        if len(recent) < 2:
            return None

        return round((len(recent) - 1) / (recent[-1] - recent[0]), 1)

    def _require_condition(self) -> asyncio.Condition:
        condition = self._condition
        if condition is None:
            raise RuntimeError("Frame delivery notifier not bound to event loop")
        return condition

    def _get_active_loop(self) -> asyncio.AbstractEventLoop | None:
        loop = self._loop
        if loop is None:
            return None
        if loop.is_closed():
            self._loop = None
            self._condition = None
            return None
        return loop

    def _schedule_frame_update(self, generation: int, published_at: float) -> None:
        asyncio.create_task(self._apply_frame_update(generation, published_at))

    def _schedule_settings_update(self, current_settings_version: int) -> None:
        asyncio.create_task(self._apply_settings_update(current_settings_version))

    async def _apply_frame_update(self, generation: int, published_at: float) -> None:
        condition = self._require_condition()
        async with condition:
            if generation <= self._frame_generation:
                return
            self._frame_generation = generation
            self._frame_times.append(published_at)
            condition.notify_all()

    async def _apply_settings_update(self, current_settings_version: int) -> None:
        condition = self._require_condition()
        async with condition:
            if current_settings_version <= self._settings_version:
                return
            self._settings_version = current_settings_version
            condition.notify_all()


class ApiRuntimeState:
    """Runtime state shared across the FastAPI application."""

    def __init__(self) -> None:
        self.backend: CameraBackendService | None = None
        self.delivery_notifier = FrameDeliveryNotifier()
        self.settings_version = 0

    def ensure_backend(self) -> CameraBackendService:
        """Initialize the backend singleton when first needed."""
        if self.backend is None:
            self.backend = get_backend_service()
            self.backend.frame_buffer.on_new_frame = self.on_new_frame
        return self.backend

    def bind_notifier_to_current_loop(self) -> None:
        """Bind the notifier to the active event loop."""
        backend = self.ensure_backend()
        snapshot = backend.frame_buffer.get_latest_snapshot()
        frame_generation = snapshot.generation if snapshot is not None else 0
        self.delivery_notifier.bind(
            asyncio.get_running_loop(),
            frame_generation=frame_generation,
            current_settings_version=self.settings_version,
        )

    def on_new_frame(self, snapshot: FrameSnapshot) -> None:
        """Publish new frames into the async delivery notifier."""
        self.delivery_notifier.publish_frame(snapshot)

    def publish_settings_update(self) -> int:
        """Increment settings version and notify listeners."""
        self.settings_version += 1
        self.delivery_notifier.publish_settings(self.settings_version)
        return self.settings_version

    def build_telemetry_payload(
        self,
        snapshot: FrameSnapshot | None,
        current_settings: dict[str, Any],
        *,
        include_settings_version: bool,
    ) -> dict[str, object]:
        """Build a telemetry payload from the latest snapshot and settings."""
        metadata = snapshot.metadata if snapshot is not None else {}
        payload: dict[str, object] = {
            "timestamp": metadata.get("timestamp"),
            "capture_ms": metadata.get("capture_duration_ms"),
            "has_frame": snapshot is not None,
            "fps": self.delivery_notifier.estimate_fps(),
            "settings": current_settings,
        }
        if include_settings_version:
            payload["settings_version"] = self.settings_version
        return payload


def initialize_runtime(app: FastAPI) -> None:
    """Attach API runtime state to the FastAPI application."""
    app.state.runtime = ApiRuntimeState()


def get_app_runtime(app: FastAPI) -> ApiRuntimeState:
    """Return the API runtime state attached to the FastAPI app."""
    return cast(ApiRuntimeState, app.state.runtime)
