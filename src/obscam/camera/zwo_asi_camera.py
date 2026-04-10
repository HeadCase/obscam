#!/usr/bin/env python3
"""ZWO ASI camera implementation for observatory monitoring."""

import io
import os
import threading
import time
import zlib
from typing import Any, override

import zwoasi as asi  # pyright: ignore[reportMissingTypeStubs]
from PIL import Image

from obscam.common.logging_config import DEBUG_MODE, get_logger

from .camera_interface import CameraInterface

logger = get_logger("zwo_asi_camera")
DEFAULT_VIDEO_MODE_THRESHOLD_MS = 200.0
DEFAULT_VIDEO_CAPTURE_TIMEOUT_PADDING_MS = 500
FRAME_SIGNATURE_SAMPLE_SIZE = 2048
PERFORMANCE_LOG_EVERY_N_FRAMES = 120


class ZwoAsiCamera(CameraInterface):
    """ZWO ASI camera implementation using the ZWO ASI SDK."""

    def __init__(self, library_path: str | None = None):
        """Initialize the ZWO ASI camera."""
        self.camera_info: dict[str, Any] = {}
        self.camera: asi.Camera | None = None
        self.is_initialized: bool = False
        self.settings_lock: threading.Lock = threading.Lock()
        self.current_capture_mode: str = ""
        self.video_mode_threshold_ms: float = DEFAULT_VIDEO_MODE_THRESHOLD_MS
        self.current_settings: dict[str, str | float | int] = {
            "exposure_ms": 200.0,
            "gain": 250,
        }
        self._capture_count = 0
        self._last_frame_signature: int | None = None
        self._duplicate_frame_streak = 0

        # Initialize the SDK
        self._init_sdk(library_path)

    def _init_sdk(self, library_path: str | None = None) -> None:
        """Initialize the ZWO ASI SDK."""
        env_filename = os.getenv("ZWO_ASI_LIB")

        try:
            if library_path:
                asi.init(library_path)
            elif env_filename:
                asi.init(env_filename)
            else:
                # Try common library paths for Raspberry Pi
                common_paths = [
                    "/usr/local/lib/libASICamera2.so",
                    "/usr/lib/libASICamera2.so",
                    "./libASICamera2.so",
                ]
                for path in common_paths:
                    if os.path.exists(path):
                        asi.init(path)
                        break
                else:
                    raise RuntimeError(
                        "ZWO ASI SDK library not found. Set ZWO_ASI_LIB "
                        "environment variable or provide library_path"
                    )

        except Exception as e:
            raise RuntimeError(f"Failed to initialize ZWO ASI SDK: {e}") from e

    @override
    def connect(self) -> bool:
        """Connect to the camera."""
        try:
            num_cameras = asi.get_num_cameras()
            if num_cameras == 0:
                print("No ZWO ASI cameras found")
                return False

            cameras_found = asi.list_cameras()
            print(f"Found {num_cameras} camera(s): {cameras_found}")

            # Look for ASI662MC specifically, otherwise use first camera
            camera_id = 0
            for i, camera_name in enumerate(cameras_found):
                if "ASI662MC" in camera_name:
                    camera_id = i
                    break

            self.camera = asi.Camera(camera_id)
            self.camera_info = self.camera.get_camera_property()

            print(f"Connected to camera: {cameras_found[camera_id]}")

            # Basic configuration
            self._configure_camera()

            self.is_initialized = True
            return True

        except Exception as e:
            print(f"Failed to connect to camera: {e}")
            return False

    def _configure_camera(self) -> None:
        """Minimal camera configuration."""
        if not self.camera:
            return

        try:
            try:
                self.camera.stop_video_capture()
                self.camera.stop_exposure()
            except asi.ZWO_Error:
                pass

            self.camera.disable_dark_subtract()
            self.camera.set_control_value(asi.ASI_GAMMA, 50)
            self.camera.set_control_value(asi.ASI_BRIGHTNESS, 50)
            self.camera.set_control_value(asi.ASI_FLIP, 0)
            self.camera.set_control_value(asi.ASI_GAIN, 250)

            # Reset capture mode
            self.current_capture_mode = ""

            print("Camera configured with minimal settings")

        except Exception as e:
            print(f"Warning: Could not configure all camera settings: {e}")

    @override
    def disconnect(self) -> None:
        """Disconnect from the camera."""
        if self.camera:
            try:
                self.camera.stop_video_capture()
                self.camera.stop_exposure()
            except asi.ZWO_Error:
                pass

            self.camera = None
            self.camera_info = {}
            self.is_initialized = False
            self.current_capture_mode = ""
            self._capture_count = 0
            self._last_frame_signature = None
            self._duplicate_frame_streak = 0
            print("Camera disconnected")

    @override
    def get_status(self) -> dict[str, Any]:
        """Get current camera status."""
        if not self.is_initialized or not self.camera:
            return {"status": "disconnected", "error": "Camera not initialized"}

        try:
            with self.settings_lock:
                settings = self.current_settings.copy()

            return {
                "status": "connected",
                "camera_model": self.camera_info.get("Name", "Unknown"),
                "current_exposure_ms": settings["exposure_ms"],
                "current_gain": settings["gain"],
            }

        except Exception as e:
            return {"status": "error", "error": str(e)}

    @override
    def capture_frame(self) -> bytes | None:
        """Capture a single frame using appropriate mode based on exposure
        time."""
        if not self.is_initialized or not self.camera:
            return None

        try:
            capture_started_at = time.perf_counter()
            with self.settings_lock:
                exposure_ms = float(self.current_settings["exposure_ms"])
                gain = int(self.current_settings["gain"])

            self._prepare_capture(exposure_ms=exposure_ms, gain=gain)

            capture_mode = (
                "video" if self._should_use_video_mode(exposure_ms) else "single"
            )
            switched_mode = self._ensure_capture_mode(capture_mode)
            acquisition_started_at = time.perf_counter()

            if capture_mode == "video":
                try:
                    img_data = self._capture_video_frame(
                        timeout_ms=self._video_capture_timeout_ms(exposure_ms)
                    )
                    if img_data is None or len(img_data) == 0:
                        raise RuntimeError("Video capture returned no data")
                except Exception as exc:
                    logger.warning(
                        "Video capture failed; falling back to single exposure",
                        error=str(exc),
                        exposure_ms=exposure_ms,
                    )
                    img_data = self._capture_single_frame()
                    capture_mode = "single"
                    switched_mode = self.current_capture_mode == "single"
            else:
                img_data = self._capture_single_frame()

            if img_data is None or len(img_data) == 0:
                return None

            acquisition_ms = round(
                (time.perf_counter() - acquisition_started_at) * 1000.0,
                2,
            )
            normalize_started_at = time.perf_counter()
            if len(img_data.shape) == 1 and self.camera_info:
                width = int(self.camera_info["MaxWidth"])
                height = int(self.camera_info["MaxHeight"])
                img_array = img_data.reshape((height, width))
            else:
                img_array = img_data
            normalize_ms = round(
                (time.perf_counter() - normalize_started_at) * 1000.0,
                2,
            )
            frame_signature = self._frame_signature(img_array)
            duplicate_frame_streak = self._update_duplicate_frame_streak(
                frame_signature
            )
            encode_started_at = time.perf_counter()
            pil_image = Image.fromarray(img_array, mode="L")

            # Encode to JPEG
            buffer = io.BytesIO()
            pil_image.save(buffer, format="JPEG", quality=85)
            encode_ms = round((time.perf_counter() - encode_started_at) * 1000.0, 2)
            total_ms = round((time.perf_counter() - capture_started_at) * 1000.0, 2)
            frame_bytes = buffer.getvalue()
            self._capture_count += 1
            self._log_capture_profile(
                exposure_ms=exposure_ms,
                capture_mode=capture_mode,
                switched_mode=switched_mode,
                acquisition_ms=acquisition_ms,
                normalize_ms=normalize_ms,
                encode_ms=encode_ms,
                total_ms=total_ms,
                duplicate_frame_streak=duplicate_frame_streak,
                frame_bytes=len(frame_bytes),
            )
            return frame_bytes

        except Exception as e:
            print(f"Frame capture failed: {e}")
            import traceback

            traceback.print_exc()
            return None

    def _should_use_video_mode(self, exposure_ms: float) -> bool:
        """Return whether the current exposure should use video capture mode."""
        return exposure_ms <= self.video_mode_threshold_ms

    def _prepare_capture(self, *, exposure_ms: float, gain: int) -> None:
        """Apply exposure, gain, and image type before raw-frame acquisition."""
        if self.camera is None:
            raise RuntimeError("Camera not initialized")

        exposure_us = int(exposure_ms * 1000)
        self.camera.set_control_value(asi.ASI_EXPOSURE, exposure_us)
        self.camera.set_control_value(asi.ASI_GAIN, gain)
        self.camera.set_image_type(asi.ASI_IMG_Y8)

    def _video_capture_timeout_ms(self, exposure_ms: float) -> int:
        """Return a bounded SDK timeout for video-frame acquisition."""
        return max(
            DEFAULT_VIDEO_CAPTURE_TIMEOUT_PADDING_MS,
            int((exposure_ms * 2.0) + DEFAULT_VIDEO_CAPTURE_TIMEOUT_PADDING_MS),
        )

    def _ensure_capture_mode(self, target_mode: str) -> bool:
        """Ensure the camera is in the requested capture mode."""
        if target_mode == "video":
            if self.current_capture_mode != "video":
                self._switch_to_video_mode()
                return True
            return False

        if self.current_capture_mode == "video":
            self._switch_to_single_mode()
            return True
        if self.current_capture_mode != "single":
            self.current_capture_mode = "single"
        return False

    def _capture_video_frame(self, timeout_ms: int | None = None):
        """Capture frame using video mode."""
        if self.camera is None:
            return None

        # Capture video frame
        return self.camera.capture_video_frame(timeout=timeout_ms)

    def _capture_single_frame(self):
        """Capture frame using single exposure mode."""
        if self.camera is None:
            return None

        # Ensure single mode is active (stop video if running)
        if self.current_capture_mode == "video":
            self._switch_to_single_mode()

        # Capture single frame
        return self.camera.capture()

    def _frame_signature(self, img_data: Any) -> int:
        """Build a lightweight fingerprint for duplicate-frame tracking."""
        flat_view = img_data.reshape(-1)
        sample = flat_view[:FRAME_SIGNATURE_SAMPLE_SIZE]
        sample_bytes = sample.tobytes()
        return zlib.crc32(sample_bytes) ^ len(flat_view)

    def _update_duplicate_frame_streak(self, frame_signature: int) -> int:
        """Track consecutive identical frame signatures."""
        if frame_signature == self._last_frame_signature:
            self._duplicate_frame_streak += 1
        else:
            self._duplicate_frame_streak = 0
            self._last_frame_signature = frame_signature
        return self._duplicate_frame_streak

    def _log_capture_profile(
        self,
        *,
        exposure_ms: float,
        capture_mode: str,
        switched_mode: bool,
        acquisition_ms: float,
        normalize_ms: float,
        encode_ms: float,
        total_ms: float,
        duplicate_frame_streak: int,
        frame_bytes: int,
    ) -> None:
        """Log high-signal timing details without spamming the hot path."""
        if switched_mode:
            logger.info(
                "Capture mode warmup frame",
                capture_mode=capture_mode,
                exposure_ms=exposure_ms,
                acquisition_ms=acquisition_ms,
                normalize_ms=normalize_ms,
                encode_ms=encode_ms,
                total_ms=total_ms,
                duplicate_frame_streak=duplicate_frame_streak,
                frame_bytes=frame_bytes,
            )
            return

        if DEBUG_MODE and (
            self._capture_count <= 3
            or self._capture_count % PERFORMANCE_LOG_EVERY_N_FRAMES == 0
        ):
            logger.debug(
                "Capture profile",
                capture_mode=capture_mode,
                exposure_ms=exposure_ms,
                acquisition_ms=acquisition_ms,
                normalize_ms=normalize_ms,
                encode_ms=encode_ms,
                total_ms=total_ms,
                duplicate_frame_streak=duplicate_frame_streak,
                frame_bytes=frame_bytes,
            )

    def _switch_to_video_mode(self):
        """Switch camera to video capture mode."""
        if self.camera is None:
            return

        try:
            # Stop any single exposure
            self.camera.stop_exposure()
        except Exception:
            pass

        # Start video mode
        self.camera.start_video_capture()
        self.current_capture_mode = "video"
        logger.info("Switched capture mode", mode="video")

    def _switch_to_single_mode(self):
        """Switch camera to single exposure mode."""
        if self.camera is None:
            return

        try:
            # Stop video capture
            self.camera.stop_video_capture()
        except Exception:
            pass

        self.current_capture_mode = "single"
        logger.info("Switched capture mode", mode="single")

    def update_settings(self, **settings: Any) -> bool:
        """Update camera settings."""
        try:
            with self.settings_lock:
                if "exposure_ms" in settings:
                    self.current_settings["exposure_ms"] = float(
                        settings["exposure_ms"]
                    )
                if "gain" in settings:
                    self.current_settings["gain"] = int(settings["gain"])
            print(f"Settings updated: {settings}")
            return True

        except Exception as e:
            print(f"Failed to update settings: {e}")
            return False

    def get_current_settings(self) -> dict[str, Any]:
        """Get current camera settings."""
        with self.settings_lock:
            return self.current_settings.copy()

    def get_control_capabilities(self) -> dict[str, Any]:
        """Get ASI camera control capabilities."""
        if not self.is_initialized or not self.camera:
            # Return generic ASI662MC capabilities when disconnected
            return {
                "exposure_ms": {"min": 0.032, "max": 30000, "type": "float"},
                "gain": {"min": 0, "max": 600, "type": "int"},
                "camera_type": "ZWO ASI (disconnected)",
            }

        try:
            control_caps = self.camera.get_controls()

            capabilities: dict[str, Any] = {"camera_type": "ZWO ASI"}

            # Map ASI control types to our interface
            if "Exposure" in control_caps:
                exp_ctrl = control_caps["Exposure"]
                capabilities["exposure_ms"] = {
                    "min": exp_ctrl["MinValue"] / 1000.0,  # Convert from microseconds
                    "max": exp_ctrl["MaxValue"] / 1000.0,
                    "type": "float",
                }

            if "Gain" in control_caps:
                gain_ctrl = control_caps["Gain"]
                capabilities["gain"] = {
                    "min": gain_ctrl["MinValue"],
                    "max": gain_ctrl["MaxValue"],
                    "type": "int",
                }

            return capabilities

        except Exception as e:
            print(f"Warning: Could not get control capabilities: {e}")
            # Return ASI662MC defaults
            return {
                "exposure_ms": {"min": 0.032, "max": 30000, "type": "float"},
                "gain": {"min": 0, "max": 600, "type": "int"},
                "camera_type": "ZWO ASI (error)",
            }
