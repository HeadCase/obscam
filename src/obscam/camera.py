import os
import io
import threading
import time
import zwoasi as asi
import numpy as np
from PIL import Image
from typing import Optional


class FastCamera:
    """Ultra-simplified camera for high-speed continuous capture."""

    def __init__(self, library_path: Optional[str] = None):
        """Initialize the camera."""
        self.camera = None
        self.camera_info = None
        self.is_initialized = False

        # Current camera settings
        self.current_exposure_us = 100000  # 100ms default
        self.current_gain = 250

        # Background capture system
        self.background_thread = None
        self.background_running = False
        self.latest_frame = None
        self.frame_timestamp = 0

        # Thread-safe settings for background capture
        self.bg_settings = {
            "exposure_us": 200000,  # 200ms default
            "gain": 250,
            "wb_r": 75,
            "wb_b": 120,
        }
        self.settings_lock = threading.Lock()
        self.frame_lock = threading.Lock()

        # Initialize the SDK
        self._init_sdk(library_path)

    def _init_sdk(self, library_path: Optional[str] = None) -> None:
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
                raise RuntimeError("No cameras found")

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

            # Basic configuration only
            self._configure_camera()

            self.is_initialized = True
            return True

        except Exception as e:
            print(f"Failed to connect to camera: {e}")
            return False

    def _configure_camera(self) -> None:
        """Minimal camera configuration."""
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
            if self.camera_info.get("IsColorCam", False):
                self.camera.set_control_value(
                    asi.ASI_WB_B, 100
                )  # Neutral white balance
                self.camera.set_control_value(asi.ASI_WB_R, 80)

            self.camera.set_control_value(asi.ASI_GAMMA, 50)  # Standard gamma
            self.camera.set_control_value(asi.ASI_BRIGHTNESS, 50)  # Standard brightness
            self.camera.set_control_value(asi.ASI_FLIP, 0)  # No flip

            print("Camera configured with minimal settings")

        except Exception as e:
            print(f"Warning: Could not configure all camera settings: {e}")

    def get_status(self) -> dict:
        """Get basic camera status."""
        if not self.is_initialized or not self.camera:
            return {"status": "disconnected", "error": "Camera not initialized"}

        try:
            current_settings = self.camera.get_control_values()

            return {
                "status": "connected",
                "camera_model": self.camera_info.get("Name", "Unknown"),
                "is_color_camera": self.camera_info.get("IsColorCam", False),
                "current_exposure_us": current_settings.get("Exposure", 100000),
                "current_gain": current_settings.get("Gain", 250),
                "current_wb_r": current_settings.get("WB_R", 80)
                if self.camera_info.get("IsColorCam", False)
                else None,
                "current_wb_b": current_settings.get("WB_B", 100)
                if self.camera_info.get("IsColorCam", False)
                else None,
            }

        except Exception as e:
            return {"status": "error", "error": str(e)}

    def start_background_capture(self) -> bool:
        """Start background capture thread for continuous frame caching."""
        if not self.is_initialized or not self.camera:
            print("Camera not initialized - cannot start background capture")
            return False

        if self.background_running:
            print("Background capture already running")
            return True

        try:
            # Enable video mode for continuous capture
            self.camera.stop_exposure()  # Stop any single exposures
            self.camera.start_video_capture()

            self.background_running = True
            self.background_thread = threading.Thread(
                target=self._background_capture_loop,
                daemon=True,
                name="CameraBackgroundCapture",
            )
            self.background_thread.start()
            print("Background capture thread started")
            return True

        except Exception as e:
            print(f"Failed to start background capture: {e}")
            self.background_running = False
            return False

    def stop_background_capture(self) -> None:
        """Stop background capture thread."""
        if self.background_running:
            self.background_running = False
            if self.background_thread:
                self.background_thread.join(timeout=2.0)

            try:
                self.camera.stop_video_capture()
            except:
                pass

            print("Background capture stopped")

    def _background_capture_loop(self) -> None:
        """Main background capture loop - runs in separate thread."""
        print("Background capture loop started")

        while self.background_running and self.is_initialized:
            try:
                # Get current settings thread-safely
                with self.settings_lock:
                    exposure_us = self.bg_settings["exposure_us"]
                    gain = self.bg_settings["gain"]
                    wb_r = self.bg_settings["wb_r"]
                    wb_b = self.bg_settings["wb_b"]

                # Apply settings to camera hardware (with error handling)
                if self.camera:
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
                        # Monochrome fallback
                        self.camera.set_image_type(asi.ASI_IMG_RAW8)
                        width = (
                            self.camera_info["MaxWidth"] if self.camera_info else 1920
                        )
                        height = (
                            self.camera_info["MaxHeight"] if self.camera_info else 1080
                        )

                    # Capture frame using video mode
                    img_buffer = self.camera.capture_video_frame()
                else:
                    raise Exception("Camera not available")

                if self.camera_info and self.camera_info.get("IsColorCam", False):
                    # Color image processing
                    img_array = np.frombuffer(img_buffer, dtype=np.uint8).reshape(
                        (height, width, 3)
                    )
                    pil_image = Image.fromarray(img_array)
                else:
                    # Monochrome image processing
                    img_array = np.frombuffer(img_buffer, dtype=np.uint8).reshape(
                        (height, width)
                    )
                    pil_image = Image.fromarray(img_array, mode="L")

                # Encode to JPEG in memory
                buffer = io.BytesIO()
                pil_image.save(buffer, format="JPEG", quality=85)
                jpeg_bytes = buffer.getvalue()

                # Store frame thread-safely
                with self.frame_lock:
                    self.latest_frame = jpeg_bytes
                    self.frame_timestamp = time.time()

            except Exception as e:
                print(f"Background capture error: {e}")
                # Brief pause before retry to avoid rapid error loops
                time.sleep(0.1)

        print("Background capture loop ended")

    def get_latest_frame(self) -> Optional[bytes]:
        """Get the latest cached frame (thread-safe, stateless)."""
        with self.frame_lock:
            return self.latest_frame

    def get_frame_info(self) -> dict:
        """Get info about the latest frame."""
        with self.frame_lock:
            return {
                "has_frame": self.latest_frame is not None,
                "frame_size": len(self.latest_frame) if self.latest_frame else 0,
                "timestamp": self.frame_timestamp,
                "background_running": self.background_running,
            }

    def update_settings(self, **settings) -> bool:
        """Update camera settings thread-safely."""
        try:
            with self.settings_lock:
                if "exposure_ms" in settings:
                    self.bg_settings["exposure_us"] = int(
                        settings["exposure_ms"] * 1000
                    )
                if "gain" in settings:
                    self.bg_settings["gain"] = int(settings["gain"])
                if "wb_r" in settings:
                    self.bg_settings["wb_r"] = int(settings["wb_r"])
                if "wb_b" in settings:
                    self.bg_settings["wb_b"] = int(settings["wb_b"])

            print(f"Settings updated: {settings}")
            return True

        except Exception as e:
            print(f"Failed to update settings: {e}")
            return False

    def get_current_settings(self) -> dict:
        """Get current camera settings thread-safely."""
        with self.settings_lock:
            return {
                "exposure_ms": self.bg_settings["exposure_us"] / 1000,
                "exposure_us": self.bg_settings["exposure_us"],
                "gain": self.bg_settings["gain"],
                "wb_r": self.bg_settings["wb_r"],
                "wb_b": self.bg_settings["wb_b"],
            }

    def disconnect(self) -> None:
        """Disconnect from the camera."""
        # Stop background capture first
        self.stop_background_capture()

        if self.camera:
            try:
                self.camera.stop_video_capture()
                self.camera.stop_exposure()
            except:
                pass

            self.camera = None
            self.is_initialized = False
            print("Camera disconnected")


# Global camera instance
_camera_instance = None


def get_camera() -> FastCamera:
    """Get the global camera instance."""
    global _camera_instance
    if _camera_instance is None:
        _camera_instance = FastCamera()
    return _camera_instance
