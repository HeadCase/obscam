#!/usr/bin/env python3
"""Backend service coordinator for camera lifecycle and recovery."""

import threading
import time
from collections.abc import Callable
from dataclasses import dataclass, field
from enum import StrEnum
from pathlib import Path
from typing import Any, TypedDict

from obscam.camera.camera_interface import CameraInterface
from obscam.common.logging_config import get_logger, log_camera_event
from obscam.core.capture_loop import ContinuousCaptureLoop
from obscam.core.frame_buffer import LatestFrameBuffer
from obscam.storage.settings_manager import SettingsManager

logger = get_logger("backend_service")

INITIAL_RECOVERY_DELAYS_S = (1.0, 2.0, 5.0)
DEGRADED_RETRY_INTERVAL_S = 30.0
MIN_FRAME_LIVE_WINDOW_S = 0.5
MAX_FRAME_LIVE_WINDOW_S = 35.0
FRAME_LIVE_PADDING_S = 1.0


class BackendLifecycleState(StrEnum):
    """Backend lifecycle states visible to the API layer."""

    STOPPED = "stopped"
    STARTING = "starting"
    RUNNING = "running"
    RECOVERING = "recovering"
    DEGRADED = "degraded"
    STOPPING = "stopping"


@dataclass(slots=True)
class BackendHealth:
    """Mutable backend lifecycle and recovery status."""

    state: BackendLifecycleState = BackendLifecycleState.STOPPED
    last_error: str | None = None
    last_transition_at: float = field(default_factory=time.time)
    consecutive_capture_failures: int = 0
    last_recovery_attempt_at: float | None = None


class BackendStateSection(TypedDict):
    """Structured backend lifecycle payload."""

    state: str
    last_error: str | None
    last_transition_at: float
    consecutive_capture_failures: int
    last_recovery_attempt_at: float | None


class CameraStateSection(TypedDict):
    """Structured camera connectivity payload."""

    connected: bool
    status: str
    model: str | None


class CaptureStateSection(TypedDict):
    """Structured capture and frame liveness payload."""

    continuous_capture: bool
    has_frame: bool
    frame_timestamp: float | None
    frame_age_seconds: float | None
    frame_is_live: bool


class SettingsStateSection(TypedDict):
    """Structured settings payload."""

    current_settings: dict[str, Any]


class BackendStatusSnapshot(TypedDict):
    """Structured backend status snapshot payload."""

    backend: BackendStateSection
    camera: CameraStateSection
    capture: CaptureStateSection
    settings: SettingsStateSection
    timestamp: float


class CameraBackendService:
    """Main backend service that owns camera operations entirely."""

    def __init__(self, camera: CameraInterface, cache_dir: Path):
        self.camera = camera
        self.cache_dir = cache_dir
        self.frame_buffer = LatestFrameBuffer()
        self.settings_manager = SettingsManager(cache_dir)
        self._health = BackendHealth()
        self._lifecycle_lock = threading.RLock()
        self._recovery_stop_event = threading.Event()
        self._recovery_request_event = threading.Event()
        self._recovery_thread: threading.Thread | None = None
        self._last_applied_settings = self.settings_manager.load_settings() or {}
        self.on_state_change: Callable[[], None] | None = None

        self.capture_loop = ContinuousCaptureLoop(
            camera,
            self.frame_buffer,
            on_capture_failure=self._on_capture_failure,
            on_capture_success=self._on_capture_success,
            on_failure_threshold=self._on_capture_failure_threshold,
        )

        logger.info(
            "Backend service initialized",
            camera_type=type(camera).__name__,
            cache_dir=str(cache_dir),
        )

    def start_backend(self) -> bool:
        """Start the backend service if it is currently stopped."""
        with self._lifecycle_lock:
            if self._health.state is not BackendLifecycleState.STOPPED:
                logger.info("Backend start requested while already active")
                return True

            self._transition_state(BackendLifecycleState.STARTING)
            self._recovery_stop_event.clear()
            self._recovery_request_event.clear()
            self.settings_manager.start()

            if not self.camera.connect():
                self._health.last_error = "Failed to connect to camera"
                log_camera_event("connection_failed")
                self._stop_background_workers_locked()
                self._transition_state(BackendLifecycleState.STOPPED)
                return False

            camera_status = self.camera.get_status()
            log_camera_event("connected", camera_status=camera_status)

            cached_settings = self._load_cached_settings()
            if cached_settings:
                logger.info("Applying cached settings", settings=cached_settings)
                if not self.camera.update_settings(**cached_settings):
                    logger.warning("Failed to apply some cached settings")

            if not self.capture_loop.start():
                self._health.last_error = "Failed to start capture loop"
                self.camera.disconnect()
                self._stop_background_workers_locked()
                self._transition_state(BackendLifecycleState.STOPPED)
                return False

            self._last_applied_settings = self.camera.get_current_settings()
            self._health.last_error = None
            self._health.consecutive_capture_failures = 0
            self._transition_state(BackendLifecycleState.RUNNING)
            log_camera_event("backend_started", status=self.get_status_snapshot())
            return True

    def stop_backend(self) -> None:
        """Stop the backend service and all background workers."""
        recovery_thread: threading.Thread | None = None
        with self._lifecycle_lock:
            if self._health.state is BackendLifecycleState.STOPPED:
                return

            self._transition_state(BackendLifecycleState.STOPPING)
            self._recovery_stop_event.set()
            self._recovery_request_event.set()
            if self._recovery_thread is not threading.current_thread():
                recovery_thread = self._recovery_thread

        self.capture_loop.stop()

        with self._lifecycle_lock:
            self.camera.disconnect()
            self._stop_background_workers_locked()
            self._health.consecutive_capture_failures = 0
            self._recovery_thread = None
            self._transition_state(BackendLifecycleState.STOPPED)
            log_camera_event("backend_stopped")

        if recovery_thread and recovery_thread.is_alive():
            recovery_thread.join(timeout=1.0)

    def request_recovery(self) -> bool:
        """Request an immediate recovery attempt when unhealthy."""
        with self._lifecycle_lock:
            if self._health.state in {
                BackendLifecycleState.STOPPED,
                BackendLifecycleState.STARTING,
                BackendLifecycleState.STOPPING,
            }:
                return False

            if self._health.state is BackendLifecycleState.RUNNING:
                return True

        return self._ensure_recovery_worker(immediate=True)

    def get_backend_state(self) -> str:
        """Return the backend lifecycle state string."""
        with self._lifecycle_lock:
            return self._health.state.value

    def get_latest_frame(self) -> bytes | None:
        """Get latest frame for HTTP response from memory buffer."""
        return self.frame_buffer.get_latest_frame()

    def get_latest_frame_with_metadata(self) -> tuple[bytes, dict[str, Any]] | None:
        """Get latest frame bytes and metadata for snapshot persistence."""
        return self.frame_buffer.get_frame_with_metadata()

    def get_frame_metadata(self) -> dict[str, Any] | None:
        """Get metadata for latest frame."""
        return self.frame_buffer.get_frame_metadata()

    def get_control_capabilities(self) -> dict[str, Any]:
        """Get the control capabilities exposed by the active camera."""
        return self.camera.get_control_capabilities()

    def update_settings(self, **settings: Any) -> bool:
        """Update camera settings and persist them."""
        if self.get_backend_state() != BackendLifecycleState.RUNNING.value:
            logger.error("Backend not running - cannot update settings")
            return False

        logger.info("Updating camera settings", new_settings=settings)
        success = self.camera.update_settings(**settings)
        if not success:
            logger.error("Failed to update camera settings", settings=settings)
            return False

        current_settings = self.camera.get_current_settings()
        self._last_applied_settings = current_settings
        self.settings_manager.save_settings_async(current_settings)
        log_camera_event("settings_updated", settings=current_settings)
        logger.info("Settings updated successfully", settings=current_settings)
        return True

    def queue_settings_update(self, settings: dict[str, Any]) -> None:
        """Queue settings for coalesced application and async persistence."""
        if self.get_backend_state() != BackendLifecycleState.RUNNING.value:
            raise RuntimeError("Backend not running - cannot update settings")

        logger.info("Queueing camera settings update", new_settings=settings)
        self.capture_loop.update_settings(settings)
        self.settings_manager.save_settings_async(
            {**self.get_current_settings(), **settings}
        )

    def get_current_settings(self) -> dict[str, Any]:
        """Get current camera settings or the last known applied settings."""
        state = self.get_backend_state()
        if state == BackendLifecycleState.STOPPED.value:
            return dict(self._load_cached_settings())
        return self.camera.get_current_settings()

    def get_last_known_settings(self) -> dict[str, Any]:
        """Return the last persisted or cached settings."""
        return dict(self._load_cached_settings())

    def get_status_snapshot(self) -> BackendStatusSnapshot:
        """Return a structured backend, camera, capture, and settings snapshot."""
        with self._lifecycle_lock:
            health = BackendHealth(
                state=self._health.state,
                last_error=self._health.last_error,
                last_transition_at=self._health.last_transition_at,
                consecutive_capture_failures=self._health.consecutive_capture_failures,
                last_recovery_attempt_at=self._health.last_recovery_attempt_at,
            )

        snapshot = self.frame_buffer.get_latest_snapshot()
        frame_timestamp: float | None = None
        frame_age_seconds: float | None = None
        exposure_ms: float | None = None
        if snapshot is not None:
            timestamp_value = snapshot.metadata.get("timestamp")
            if isinstance(timestamp_value, int | float):
                frame_timestamp = float(timestamp_value)
                frame_age_seconds = max(0.0, time.time() - frame_timestamp)

            exposure_value = snapshot.metadata.get("exposure_ms")
            if isinstance(exposure_value, int | float):
                exposure_ms = float(exposure_value)

        current_settings = self.get_current_settings()
        if exposure_ms is None:
            current_exposure = current_settings.get("exposure_ms")
            if isinstance(current_exposure, int | float):
                exposure_ms = float(current_exposure)

        try:
            raw_camera_status = self.camera.get_status()
        except Exception as exc:
            raw_camera_status = {"status": "error", "error": str(exc)}

        camera_connected = raw_camera_status.get("status") == "connected"
        frame_is_live = (
            health.state is BackendLifecycleState.RUNNING
            and snapshot is not None
            and frame_age_seconds is not None
            and frame_age_seconds <= self._frame_live_window_s(exposure_ms)
        )

        return {
            "backend": {
                "state": health.state.value,
                "last_error": health.last_error,
                "last_transition_at": health.last_transition_at,
                "consecutive_capture_failures": health.consecutive_capture_failures,
                "last_recovery_attempt_at": health.last_recovery_attempt_at,
            },
            "camera": {
                "connected": camera_connected,
                "status": raw_camera_status.get("status", "unknown"),
                "model": raw_camera_status.get("camera_model")
                or raw_camera_status.get("camera_type"),
            },
            "capture": {
                "continuous_capture": self.capture_loop.is_running(),
                "has_frame": snapshot is not None,
                "frame_timestamp": frame_timestamp,
                "frame_age_seconds": round(frame_age_seconds, 3)
                if frame_age_seconds is not None
                else None,
                "frame_is_live": frame_is_live,
            },
            "settings": {
                "current_settings": current_settings,
            },
            "timestamp": time.time(),
        }

    def is_started(self) -> bool:
        """Return whether the backend is not fully stopped."""
        return self.get_backend_state() != BackendLifecycleState.STOPPED.value

    def __enter__(self):
        """Context manager entry."""
        self.start_backend()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        """Context manager exit."""
        self.stop_backend()

    def shutdown_gracefully(self) -> None:
        """Gracefully stop the backend, persisting current settings first."""
        if self.get_backend_state() == BackendLifecycleState.STOPPED.value:
            logger.info("Backend service already stopped")
            return

        logger.info("Performing graceful shutdown of backend service")
        try:
            current_settings = self.camera.get_current_settings()
            self._last_applied_settings = current_settings
            self.settings_manager.save_settings_sync(current_settings)
            logger.info("Settings saved successfully during shutdown")
        except Exception as exc:
            logger.error("Failed to save settings during shutdown", error=str(exc))

        self.stop_backend()
        logger.info("Backend service graceful shutdown completed")

    def _transition_state(self, new_state: BackendLifecycleState) -> None:
        old_state = self._health.state
        self._health.state = new_state
        self._health.last_transition_at = time.time()
        if old_state is not new_state:
            logger.info(
                "Backend lifecycle transition",
                from_state=old_state.value,
                to_state=new_state.value,
            )
            log_camera_event(
                "backend_state_changed",
                from_state=old_state.value,
                to_state=new_state.value,
            )
            if self.on_state_change is not None:
                try:
                    self.on_state_change()
                except Exception as exc:
                    logger.error("State change callback failed", error=str(exc))

    def _load_cached_settings(self) -> dict[str, Any]:
        if self._last_applied_settings:
            return dict(self._last_applied_settings)

        cached_settings = self.settings_manager.load_settings() or {}
        self._last_applied_settings = dict(cached_settings)
        return dict(self._last_applied_settings)

    def _frame_live_window_s(self, exposure_ms: float | None) -> float:
        exposure_s = 0.0 if exposure_ms is None else exposure_ms / 1000.0
        return min(
            MAX_FRAME_LIVE_WINDOW_S,
            max(MIN_FRAME_LIVE_WINDOW_S, exposure_s + FRAME_LIVE_PADDING_S),
        )

    def _stop_background_workers_locked(self) -> None:
        self.settings_manager.stop()

    def _on_capture_failure(
        self,
        error_message: str,
        consecutive_failures: int,
    ) -> None:
        with self._lifecycle_lock:
            if self._health.state in {
                BackendLifecycleState.STOPPED,
                BackendLifecycleState.STOPPING,
            }:
                return
            self._health.consecutive_capture_failures = consecutive_failures
            self._health.last_error = error_message

    def _on_capture_success(self) -> None:
        with self._lifecycle_lock:
            self._health.consecutive_capture_failures = 0
            if self._health.state is BackendLifecycleState.RUNNING:
                self._health.last_error = None

    def _on_capture_failure_threshold(
        self, error_message: str, consecutive_failures: int
    ) -> None:
        with self._lifecycle_lock:
            if self._health.state in {
                BackendLifecycleState.STOPPED,
                BackendLifecycleState.STOPPING,
            }:
                return
            self._health.consecutive_capture_failures = consecutive_failures
            self._health.last_error = error_message
            if self._health.state is BackendLifecycleState.RUNNING:
                self._transition_state(BackendLifecycleState.RECOVERING)

        self._ensure_recovery_worker(immediate=False)

    def _ensure_recovery_worker(self, *, immediate: bool) -> bool:
        thread_to_start: threading.Thread | None = None
        with self._lifecycle_lock:
            if self._health.state in {
                BackendLifecycleState.STOPPED,
                BackendLifecycleState.STARTING,
                BackendLifecycleState.STOPPING,
            }:
                return False

            if self._health.state in {
                BackendLifecycleState.RUNNING,
                BackendLifecycleState.DEGRADED,
            }:
                self._transition_state(BackendLifecycleState.RECOVERING)

            if immediate:
                self._recovery_request_event.set()

            if self._recovery_thread is not None and self._recovery_thread.is_alive():
                return True

            self._recovery_stop_event.clear()
            thread_to_start = threading.Thread(
                target=self._recovery_worker,
                daemon=True,
                name="BackendRecoveryWorker",
            )
            self._recovery_thread = thread_to_start

        if thread_to_start is not None:
            thread_to_start.start()
        return True

    def _recovery_worker(self) -> None:
        logger.warning("Backend recovery worker started")
        try:
            for delay_s in INITIAL_RECOVERY_DELAYS_S:
                if self._wait_for_recovery_window(delay_s):
                    return
                if self._attempt_recovery_once():
                    return

            with self._lifecycle_lock:
                if self._health.state not in {
                    BackendLifecycleState.STOPPING,
                    BackendLifecycleState.STOPPED,
                }:
                    self._transition_state(BackendLifecycleState.DEGRADED)
                    logger.warning("Backend recovery exhausted initial retries")

            while not self._recovery_stop_event.is_set():
                self._recovery_request_event.wait(timeout=DEGRADED_RETRY_INTERVAL_S)
                self._recovery_request_event.clear()
                if self._recovery_stop_event.is_set():
                    return
                if self._attempt_recovery_once():
                    return
        finally:
            with self._lifecycle_lock:
                if self._recovery_thread is threading.current_thread():
                    self._recovery_thread = None
            logger.warning("Backend recovery worker stopped")

    def _wait_for_recovery_window(self, delay_s: float) -> bool:
        self._recovery_request_event.wait(timeout=delay_s)
        self._recovery_request_event.clear()
        return self._recovery_stop_event.is_set()

    def _attempt_recovery_once(self) -> bool:
        if self._recovery_stop_event.is_set():
            return False

        with self._lifecycle_lock:
            if self._health.state in {
                BackendLifecycleState.STOPPING,
                BackendLifecycleState.STOPPED,
            }:
                return False
            self._transition_state(BackendLifecycleState.RECOVERING)
            self._health.last_recovery_attempt_at = time.time()

        logger.warning("Attempting backend recovery")
        self.capture_loop.stop()
        if self._recovery_stop_event.is_set():
            return False

        self.camera.disconnect()
        if self._recovery_stop_event.is_set():
            return False

        if not self.camera.connect():
            with self._lifecycle_lock:
                self._health.last_error = "Recovery reconnect failed"
            logger.warning("Recovery reconnect failed")
            return False

        cached_settings = self._load_cached_settings()
        if cached_settings and not self.camera.update_settings(**cached_settings):
            logger.warning("Failed to reapply cached settings during recovery")

        self._last_applied_settings = self.camera.get_current_settings()
        if not self.capture_loop.start():
            self.camera.disconnect()
            with self._lifecycle_lock:
                self._health.last_error = "Recovery failed to restart capture loop"
            logger.warning("Recovery failed to restart capture loop")
            return False

        with self._lifecycle_lock:
            self._health.consecutive_capture_failures = 0
            self._health.last_error = None
            self._transition_state(BackendLifecycleState.RUNNING)

        logger.info("Backend recovery succeeded")
        return True
