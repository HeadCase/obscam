#!/usr/bin/env python3
"""Continuous capture loop with queue-based thread-safe coordination."""

import queue
import threading
import time
from collections.abc import Callable
from typing import Any

from obscam.camera.camera_interface import CameraInterface
from obscam.common.logging_config import get_logger, log_camera_event
from obscam.core.frame_buffer import LatestFrameBuffer

logger = get_logger("capture_loop")

FAILURE_THRESHOLD = 5
MIN_STOP_TIMEOUT_S = 0.5
MAX_STOP_TIMEOUT_S = 35.0
STOP_TIMEOUT_PADDING_S = 1.0


class ContinuousCaptureLoop:
    """Manages continuous capture using queue-based coordination."""

    def __init__(
        self,
        camera: CameraInterface,
        frame_buffer: LatestFrameBuffer,
        *,
        on_capture_failure: Callable[[str, int], None] | None = None,
        on_capture_success: Callable[[], None] | None = None,
        on_failure_threshold: Callable[[str, int], None] | None = None,
    ):
        self.camera = camera
        self.frame_buffer = frame_buffer
        self.on_capture_failure = on_capture_failure
        self.on_capture_success = on_capture_success
        self.on_failure_threshold = on_failure_threshold

        self._settings_queue: queue.Queue[dict[str, Any]] = queue.Queue()
        self._control_queue: queue.Queue[str] = queue.Queue()
        self.capture_thread: threading.Thread | None = None
        self.running = False

        logger.info("Capture loop initialized")

    def start(self) -> bool:
        """Start the continuous capture loop."""
        if self.is_running():
            logger.info("Capture loop already running")
            return True

        if self.camera.get_status().get("status") != "connected":
            logger.error("Camera not connected - cannot start capture loop")
            return False

        self._clear_control_queue()
        self.running = True
        self.capture_thread = threading.Thread(
            target=self._capture_worker,
            daemon=True,
            name="ContinuousCaptureLoop",
        )
        self.capture_thread.start()
        log_camera_event("capture_loop_started")
        return True

    def stop(self) -> None:
        """Stop the continuous capture loop gracefully."""
        thread = self.capture_thread
        if not self.running and (thread is None or not thread.is_alive()):
            return

        logger.info("Stopping capture loop")
        self.running = False

        try:
            self._control_queue.put_nowait("stop")
        except queue.Full:
            pass

        try:
            self.camera.interrupt_capture()
        except Exception as exc:
            logger.warning("Failed to interrupt active capture", error=str(exc))

        if thread and thread is not threading.current_thread():
            timeout_s = self._stop_join_timeout_s()
            thread.join(timeout=timeout_s)
            if thread.is_alive():
                logger.warning(
                    "Capture thread did not stop cleanly within timeout",
                    timeout_s=timeout_s,
                )
            else:
                logger.info("Capture thread stopped successfully")

        log_camera_event("capture_loop_stopped")

    def update_settings(self, settings: dict[str, Any]) -> None:
        """Queue settings update for capture loop."""
        try:
            while not self._settings_queue.empty():
                try:
                    self._settings_queue.get_nowait()
                except queue.Empty:
                    break

            self._settings_queue.put_nowait(settings)
            logger.debug("Settings update queued", settings=settings)
        except queue.Full:
            logger.warning("Settings queue full - dropping settings update")

    def is_running(self) -> bool:
        """Check if capture loop is running."""
        thread = self.capture_thread
        return self.running and thread is not None and thread.is_alive()

    def _clear_control_queue(self) -> None:
        """Clear any leftover commands in the control queue."""
        cleared_count = 0
        while not self._control_queue.empty():
            try:
                self._control_queue.get_nowait()
                cleared_count += 1
            except queue.Empty:
                break
        if cleared_count > 0:
            logger.debug(
                "Cleared leftover control commands",
                cleared_count=cleared_count,
            )

    def _stop_join_timeout_s(self) -> float:
        """Return an exposure-aware timeout for stopping the worker."""
        current_settings = self.camera.get_current_settings()
        exposure_ms = current_settings.get("exposure_ms", 0.0)
        exposure_s = (
            float(exposure_ms) / 1000.0 if isinstance(exposure_ms, int | float) else 0.0
        )
        return min(
            MAX_STOP_TIMEOUT_S,
            max(MIN_STOP_TIMEOUT_S, exposure_s + STOP_TIMEOUT_PADDING_S),
        )

    def _report_capture_failure(self, message: str, consecutive_failures: int) -> None:
        logger.error(
            "Capture loop error",
            error=message,
            consecutive_capture_failures=consecutive_failures,
        )
        if self.on_capture_failure is not None:
            self.on_capture_failure(message, consecutive_failures)

        if (
            consecutive_failures >= FAILURE_THRESHOLD
            and self.on_failure_threshold is not None
        ):
            self.on_failure_threshold(message, consecutive_failures)

    def _capture_worker(self) -> None:
        """Main capture worker thread."""
        logger.info("Capture loop thread started")

        settings = self.camera.get_current_settings()
        frame_count = 0
        consecutive_failures = 0

        try:
            while self.running:
                try:
                    command = self._control_queue.get_nowait()
                    if command == "stop":
                        logger.debug("Received stop command")
                        break
                except queue.Empty:
                    pass

                try:
                    new_settings = self._settings_queue.get_nowait()
                    logger.info("Applying settings update", settings=new_settings)
                    success = self.camera.update_settings(**new_settings)
                    if success:
                        settings = self.camera.get_current_settings()
                        log_camera_event("settings_updated", settings=new_settings)
                    else:
                        logger.error("Failed to apply settings", settings=new_settings)
                except queue.Empty:
                    pass

                capture_started_at = time.perf_counter()
                try:
                    frame_bytes = self.camera.capture_frame()
                except Exception as exc:
                    consecutive_failures += 1
                    self._report_capture_failure(str(exc), consecutive_failures)
                    if consecutive_failures >= FAILURE_THRESHOLD:
                        break
                    time.sleep(0.2)
                    continue

                capture_duration_ms = round(
                    (time.perf_counter() - capture_started_at) * 1000.0,
                    2,
                )

                if not self.running:
                    break

                if frame_bytes:
                    metadata = {
                        **settings,
                        "timestamp": time.time(),
                        "capture_duration_ms": capture_duration_ms,
                    }
                    self.frame_buffer.update_frame(frame_bytes, metadata)
                    frame_count += 1
                    if consecutive_failures > 0 and self.on_capture_success is not None:
                        self.on_capture_success()
                    consecutive_failures = 0
                else:
                    consecutive_failures += 1
                    self._report_capture_failure(
                        "Frame capture returned no data",
                        consecutive_failures,
                    )
                    if consecutive_failures >= FAILURE_THRESHOLD:
                        break
                    time.sleep(0.2)
                    continue

                time.sleep(0.001)
        finally:
            self.running = False
            logger.info("Capture loop thread ended", final_frame_count=frame_count)
