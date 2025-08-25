#!/usr/bin/env python3
"""ZWO ASI camera implementation for observatory monitoring."""

import io
import os
import threading
import time
from typing import Any

import numpy as np
import zwoasi as asi  # pyright: ignore[reportMissingTypeStubs]
from PIL import Image

from .camera_interface import CameraInterface, FrameMetadata


class ZwoAsiCamera(CameraInterface):
    """ZWO ASI camera implementation using the ZWO ASI SDK."""

    def __init__(self, library_path: str | None = None):
        """Initialize the ZWO ASI camera."""
        self.camera: Any = None
        self.camera_info: dict[str, Any] | None = None
        self.is_initialized = False

        # Current camera settings
        self.current_settings = {
            "exposure_ms": 200.0,  # 200ms default
            "gain": 600,
            "wb_r": 70,
            "wb_b": 70,
        }
        self.settings_lock = threading.Lock()

        # Frame caching
        self.latest_frame: bytes | None = None
        self.frame_metadata: FrameMetadata | None = None
        self.frame_lock = threading.Lock()

        # Continuous capture control
        self.capture_thread: threading.Thread | None = None
        self.capture_running = False

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
                        "ZWO ASI SDK library not found. Set ZWO_ASI_LIB environment variable or provide library_path"
                    )

        except Exception as e:
            raise RuntimeError(f"Failed to initialize ZWO ASI SDK: {e}")

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
            # Stop any ongoing operations
            try:
                self.camera.stop_video_capture()
                self.camera.stop_exposure()
            except:
                pass

            # Disable dark subtract
            self.camera.disable_dark_subtract()

            # Basic color camera settings
            if self.camera_info and self.camera_info.get("IsColorCam", False):
                self.camera.set_control_value(asi.ASI_WB_B, 100)
                self.camera.set_control_value(asi.ASI_WB_R, 80)

            self.camera.set_control_value(asi.ASI_GAMMA, 50)
            self.camera.set_control_value(asi.ASI_BRIGHTNESS, 50)
            self.camera.set_control_value(asi.ASI_FLIP, 0)

            print("Camera configured with minimal settings")

        except Exception as e:
            print(f"Warning: Could not configure all camera settings: {e}")

    def disconnect(self) -> None:
        """Disconnect from the camera."""
        self.stop_continuous_capture()

        if self.camera:
            try:
                self.camera.stop_video_capture()
                self.camera.stop_exposure()
            except:
                pass

            self.camera = None
            self.is_initialized = False
            print("Camera disconnected")

    def get_status(self) -> dict[str, Any]:
        """Get current camera status."""
        if not self.is_initialized or not self.camera:
            return {"status": "disconnected", "error": "Camera not initialized"}

        try:
            current_values = self.camera.get_control_values()

            with self.settings_lock:
                settings = self.current_settings.copy()

            return {
                "status": "connected",
                "camera_model": self.camera_info.get("Name", "Unknown")
                if self.camera_info
                else "Unknown",
                "is_color_camera": self.camera_info.get("IsColorCam", False)
                if self.camera_info
                else False,
                "current_exposure_ms": settings["exposure_ms"],
                "current_gain": settings["gain"],
                "current_wb_r": settings.get("wb_r"),
                "current_wb_b": settings.get("wb_b"),
                "continuous_capture": self.capture_running,
            }

        except Exception as e:
            return {"status": "error", "error": str(e)}

    def start_continuous_capture(self) -> bool:
        """Start continuous capture loop."""
        if not self.is_initialized or not self.camera:
            print("Camera not initialized - cannot start continuous capture")
            return False

        if self.capture_running:
            print("Continuous capture already running")
            return True

        try:
            # Start video mode for continuous capture
            self.camera.stop_exposure()
            self.camera.start_video_capture()

            self.capture_running = True
            self.capture_thread = threading.Thread(
                target=self._capture_loop,
                daemon=True,
                name="ZwoAsiCaptureLoop",
            )
            self.capture_thread.start()
            print("Continuous capture started")
            return True

        except Exception as e:
            print(f"Failed to start continuous capture: {e}")
            self.capture_running = False
            return False

    def stop_continuous_capture(self) -> None:
        """Stop continuous capture loop."""
        if self.capture_running:
            self.capture_running = False
            if self.capture_thread:
                self.capture_thread.join(timeout=2.0)

            try:
                if self.camera:
                    self.camera.stop_video_capture()
            except:
                pass

            print("Continuous capture stopped")

    def _capture_loop(self) -> None:
        """Main capture loop - runs in separate thread."""
        print("Capture loop started")

        while self.capture_running and self.is_initialized and self.camera:
            try:
                # Get current settings
                with self.settings_lock:
                    exposure_us = int(self.current_settings["exposure_ms"] * 1000)
                    gain = int(self.current_settings["gain"])
                    wb_r = int(self.current_settings.get("wb_r", 75))
                    wb_b = int(self.current_settings.get("wb_b", 120))

                # Apply settings to camera
                self.camera.set_control_value(asi.ASI_EXPOSURE, exposure_us)
                self.camera.set_control_value(asi.ASI_GAIN, gain)

                # Apply white balance for color cameras
                if self.camera_info and self.camera_info.get("IsColorCam", False):
                    self.camera.set_control_value(asi.ASI_WB_R, wb_r)
                    self.camera.set_control_value(asi.ASI_WB_B, wb_b)
                    self.camera.set_image_type(asi.ASI_IMG_RGB24)
                    width = self.camera_info["MaxWidth"]
                    height = self.camera_info["MaxHeight"]
                else:
                    self.camera.set_image_type(asi.ASI_IMG_RAW8)
                    width = self.camera_info["MaxWidth"] if self.camera_info else 1920
                    height = self.camera_info["MaxHeight"] if self.camera_info else 1080

                # Capture frame
                img_buffer = self.camera.capture_video_frame()
                capture_time = time.time()

                # Process image
                if self.camera_info and self.camera_info.get("IsColorCam", False):
                    img_array = np.frombuffer(img_buffer, dtype=np.uint8).reshape(
                        (height, width, 3)
                    )
                    pil_image = Image.fromarray(img_array)
                else:
                    img_array = np.frombuffer(img_buffer, dtype=np.uint8).reshape(
                        (height, width)
                    )
                    pil_image = Image.fromarray(img_array, mode="L")

                # Encode to JPEG
                buffer = io.BytesIO()
                pil_image.save(buffer, format="JPEG", quality=85)
                jpeg_bytes = buffer.getvalue()

                # Store frame with metadata
                with self.frame_lock:
                    self.latest_frame = jpeg_bytes
                    self.frame_metadata = FrameMetadata(
                        exposure_ms=self.current_settings["exposure_ms"],
                        gain=gain,
                        wb_r=wb_r
                        if self.camera_info
                        and self.camera_info.get("IsColorCam", False)
                        else None,
                        wb_b=wb_b
                        if self.camera_info
                        and self.camera_info.get("IsColorCam", False)
                        else None,
                        timestamp=capture_time,
                    )

            except Exception as e:
                print(f"Capture error: {e}")
                time.sleep(0.1)  # Brief pause before retry

        print("Capture loop ended")

    def get_latest_frame(self) -> bytes | None:
        """Get the latest cached frame."""
        with self.frame_lock:
            return self.latest_frame

    def get_frame_metadata(self) -> FrameMetadata | None:
        """Get metadata for the latest frame."""
        with self.frame_lock:
            return self.frame_metadata

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
                if "wb_r" in settings:
                    self.current_settings["wb_r"] = int(settings["wb_r"])
                if "wb_b" in settings:
                    self.current_settings["wb_b"] = int(settings["wb_b"])

            print(f"Settings updated: {settings}")
            return True

        except Exception as e:
            print(f"Failed to update settings: {e}")
            return False

    def get_current_settings(self) -> dict[str, Any]:
        """Get current camera settings."""
        with self.settings_lock:
            return self.current_settings.copy()
