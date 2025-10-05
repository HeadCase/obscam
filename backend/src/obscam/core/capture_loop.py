#!/usr/bin/env python3
"""Continuous capture loop with queue-based thread-safe coordination."""

import queue
import threading
import time
from typing import Any

from obscam.camera.camera_interface import CameraInterface
from obscam.core.frame_buffer import LatestFrameBuffer
from obscam.common.logging_config import get_logger, log_camera_event, log_performance

logger = get_logger("capture_loop")


class ContinuousCaptureLoop:
    """Manages continuous capture using queue-based coordination - minimal locking."""

    def __init__(self, camera: CameraInterface, frame_buffer: LatestFrameBuffer):
        self.camera = camera
        self.frame_buffer = frame_buffer

        # Thread-safe queues for coordination
        self._settings_queue: queue.Queue[dict[str, Any]] = queue.Queue()
        self._control_queue: queue.Queue[str] = queue.Queue()

        # Thread management
        self.capture_thread: threading.Thread | None = None
        self.running = False

        logger.info("Capture loop initialized")

    def start(self) -> bool:
        """Start the continuous capture loop."""
        if self.running:
            logger.info("Capture loop already running")
            return True

        if self.camera.get_status().get("status") != "connected":
            logger.error("Camera not connected - cannot start capture loop")
            return False

        # Clear any leftover control commands before starting
        self._clear_control_queue()

        self.running = True
        self.capture_thread = threading.Thread(
            target=self._capture_worker, daemon=True, name="ContinuousCaptureLoop"
        )
        self.capture_thread.start()
        log_camera_event("capture_loop_started")
        return True

    def stop(self) -> None:
        """Stop the continuous capture loop gracefully."""
        if self.running:
            logger.info("Stopping capture loop...")
            self.running = False

            # Signal worker thread to stop
            try:
                self._control_queue.put_nowait("stop")
            except queue.Full:
                pass

            if self.capture_thread:
                self.capture_thread.join(timeout=2.0)
                if self.capture_thread.is_alive():
                    logger.warning("Capture thread did not stop cleanly within timeout")
                else:
                    logger.info("Capture thread stopped successfully")

            log_camera_event("capture_loop_stopped")

    def update_settings(self, settings: dict[str, Any]) -> None:
        """Queue settings update for capture loop."""
        try:
            # Clear old settings updates and queue the latest
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
        return self.running and (self.capture_thread is not None)

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
            logger.debug(f"Cleared {cleared_count} leftover control commands")

    def _capture_worker(self) -> None:
        """Main capture worker - single thread, no locks needed internally."""
        logger.info("Capture loop thread started")

        settings = self.camera.get_current_settings()
        frame_count = 0
        error_count = 0

        while self.running:
            try:
                # Check for control commands (non-blocking)
                try:
                    command = self._control_queue.get_nowait()
                    if command == "stop":
                        logger.debug("Received stop command")
                        break
                except queue.Empty:
                    pass

                # Check for settings updates (non-blocking)
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

                # Capture frame with timing
                frame_bytes = self.camera.capture_frame()

                if frame_bytes:
                    metadata = {
                        **settings,
                        "timestamp": time.time(),
                        "capture_duration_ms": settings["exposure_ms"],
                    }

                    # Update frame buffer (thread-safe queue internally)
                    self.frame_buffer.update_frame(frame_bytes, metadata)

                    frame_count += 1

                else:
                    logger.warning("Frame capture returned no data")

                time.sleep(0.001)  # yield to other threads

            except Exception as e:
                error_count += 1
                logger.error(
                    "Capture loop error",
                    error=str(e),
                    error_count=error_count,
                    frame_count=frame_count,
                )

                # Exponential backoff on repeated errors
                if error_count > 5:
                    sleep_time = min(5.0, 0.5 * (2 ** (error_count - 5)))
                    logger.warning(f"Multiple errors, backing off for {sleep_time}s")
                    time.sleep(sleep_time)
                else:
                    time.sleep(1.0)

        logger.info("Capture loop thread ended", final_frame_count=frame_count)
