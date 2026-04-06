#!/usr/bin/env python3
"""Backend service coordinator for camera system."""

from pathlib import Path
from typing import Any

from obscam.camera.camera_interface import CameraInterface
from obscam.common.logging_config import get_logger, log_camera_event
from obscam.core.capture_loop import ContinuousCaptureLoop
from obscam.core.frame_buffer import LatestFrameBuffer
from obscam.storage.settings_manager import SettingsManager

logger = get_logger("backend_service")


class CameraBackendService:
    """Main backend service that owns camera operations entirely.

    This service orchestrates all camera-related components and provides
    a clean API for the web layer. It uses queue-based coordination
    for thread safety with minimal locking.

    Responsibilities:
    - Camera connection and lifecycle management
    - Continuous capture orchestration
    - Settings persistence and management
    - Frame serving to web layer from memory buffer
    """

    def __init__(self, camera: CameraInterface, cache_dir: Path):
        self.camera = camera
        self.cache_dir = cache_dir
        self._started = False

        # Initialize core components
        self.frame_buffer = LatestFrameBuffer()
        self.capture_loop = ContinuousCaptureLoop(camera, self.frame_buffer)
        self.settings_manager = SettingsManager(cache_dir)

        logger.info(
            "Backend service initialized",
            camera_type=type(camera).__name__,
            cache_dir=str(cache_dir),
        )

    def start_backend(self) -> bool:
        """Start the complete backend service - camera + capture + persistence."""
        if self._started:
            logger.info("Backend service already started")
            return True

        logger.info("Starting camera backend service")

        # Start background workers
        self.settings_manager.start()

        # Connect to camera hardware
        if not self.camera.connect():
            logger.error("Failed to connect to camera")
            log_camera_event("connection_failed")
            self._stop_background_workers()
            return False

        camera_status = self.camera.get_status()
        log_camera_event("connected", camera_status=camera_status)

        # Apply cached settings if available
        cached_settings = self.settings_manager.load_settings()
        if cached_settings:
            logger.info("Applying cached settings", settings=cached_settings)
            success = self.camera.update_settings(**cached_settings)
            if success:
                log_camera_event("settings_applied", settings=cached_settings)
            else:
                logger.warning("Failed to apply some cached settings")

        # Start continuous capture loop
        if not self.capture_loop.start():
            logger.error("Failed to start capture loop")
            self.camera.disconnect()
            self._stop_background_workers()
            return False

        self._started = True
        logger.info("Backend service started successfully")
        log_camera_event("backend_started", status=self.get_status())
        return True

    def stop_backend(self) -> None:
        """Stop the complete backend service."""
        if not self._started:
            return

        logger.info("Stopping backend service")

        # Stop capture loop first
        self.capture_loop.stop()

        # Disconnect camera
        self.camera.disconnect()

        # Stop background workers
        self._stop_background_workers()

        self._started = False
        log_camera_event("backend_stopped")

    def _stop_background_workers(self) -> None:
        """Stop all background worker threads."""
        self.settings_manager.stop()

    # API Methods for Web Layer - Clean interface with no threading concerns

    def get_latest_frame(self) -> bytes | None:
        """Get latest frame for HTTP response from memory buffer."""
        if not self._started:
            logger.warning("Backend not started - cannot serve frame")
            return None

        frame = self.frame_buffer.get_latest_frame()
        if frame:
            logger.debug("Latest frame served from memory", frame_size=len(frame))
        else:
            logger.debug("No frame available in memory buffer")
        return frame

    def get_latest_frame_with_metadata(self) -> tuple[bytes, dict[str, Any]] | None:
        """Get latest frame bytes and metadata for snapshot persistence."""
        if not self._started:
            logger.warning("Backend not started - cannot serve frame with metadata")
            return None

        frame_data = self.frame_buffer.get_frame_with_metadata()
        if frame_data:
            logger.debug(
                "Latest frame with metadata served from memory",
                frame_size=len(frame_data[0]),
            )
        else:
            logger.debug("No frame with metadata available in memory buffer")
        return frame_data

    def get_frame_metadata(self) -> dict[str, Any] | None:
        """Get metadata for latest frame."""
        if not self._started:
            return None
        return self.frame_buffer.get_frame_metadata()

    def get_control_capabilities(self) -> dict[str, Any]:
        """Get the control capabilities exposed by the active camera."""
        return self.camera.get_control_capabilities()

    def update_settings(self, **settings: Any) -> bool:
        """Update camera settings and persist them.

        This method coordinates between the camera hardware, capture
        loop, and settings persistence.
        """
        if not self._started:
            logger.error("Backend not started - cannot update settings")
            return False

        logger.info("Updating camera settings", new_settings=settings)

        # Apply settings to camera hardware
        success = self.camera.update_settings(**settings)

        if success:
            # Get updated settings from camera
            current_settings = self.camera.get_current_settings()

            # Persist settings asynchronously
            if success:
                self.settings_manager.save_settings_async(current_settings)
                log_camera_event("settings_updated", settings=current_settings)
                logger.info("Settings updated successfully", settings=current_settings)
        else:
            logger.error("Failed to update camera settings", settings=settings)

        return success

    def queue_settings_update(self, settings: dict[str, Any]) -> None:
        """Queue settings for coalesced application and async persistence."""
        if not self._started:
            raise RuntimeError("Backend not started - cannot update settings")

        logger.info("Queueing camera settings update", new_settings=settings)
        self.capture_loop.update_settings(settings)
        self.settings_manager.save_settings_async(
            {**self.get_current_settings(), **settings}
        )

    def get_current_settings(self) -> dict[str, Any]:
        """Get current camera settings."""
        if not self._started:
            return {}
        return self.camera.get_current_settings()

    def get_status(self) -> dict[str, Any]:
        """Get comprehensive backend status for monitoring/debugging."""
        base_status = {
            "backend_service": "running" if self._started else "stopped",
            "continuous_capture": False,
            "has_frame": False,
        }

        if not self._started:
            return base_status

        # Get camera status
        try:
            camera_status = self.camera.get_status()
            base_status.update(camera_status)
        except Exception as e:
            logger.error("Failed to get camera status", error=str(e))
            base_status["camera_error"] = str(e)

        # Add backend-specific status
        base_status.update(
            {
                "continuous_capture": self.capture_loop.is_running(),
                "has_frame": self.frame_buffer.has_frame(),
            }
        )

        return base_status

    def is_started(self) -> bool:
        """Check if backend service is started."""
        return self._started

    # Lifecycle management for web application integration

    def __enter__(self):
        """Context manager entry."""
        self.start_backend()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        """Context manager exit."""
        self.stop_backend()

    def shutdown_gracefully(self) -> None:
        """Graceful shutdown with final settings save."""
        if self._started:
            logger.info("Performing graceful shutdown of backend service")

            # Save current settings synchronously before shutdown
            try:
                current_settings = self.camera.get_current_settings()
                self.settings_manager.save_settings_sync(current_settings)
                logger.info("Settings saved successfully during shutdown")
            except Exception as e:
                logger.error("Failed to save settings during shutdown", error=str(e))

            # Stop the backend
            self.stop_backend()
            logger.info("Backend service graceful shutdown completed")
        else:
            logger.info("Backend service already stopped")
