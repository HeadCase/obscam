import os
import io
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

        # Master parameter storage - separate from hardware settings
        self.master_exposure_us = 200000  # 200ms default
        self.master_gain = 250
        self.master_wb_r = 75  # Red white balance (reduce for less magenta)
        self.master_wb_b = 120  # Blue white balance (increase for less magenta)

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

    def capture_fast_master(
        self,
        exposure_us: int,
        gain: int,
        wb_r: Optional[int] = None,
        wb_b: Optional[int] = None,
    ) -> Optional[bytes]:
        """Master capture: set exposure/gain/white balance, capture, return JPEG bytes directly.

        Args:
            exposure_us: Exposure time in microseconds
            gain: Camera gain value
            wb_r: Red white balance (50-150, optional)
            wb_b: Blue white balance (50-150, optional)

        Returns:
            JPEG image as bytes, or None if capture failed
        """
        if not self.is_initialized or not self.camera:
            print("Camera not initialized")
            return None

        try:
            # Update camera hardware parameters
            self.camera.set_control_value(asi.ASI_EXPOSURE, exposure_us)
            self.camera.set_control_value(asi.ASI_GAIN, gain)

            # Update white balance if provided and camera supports color
            if self.camera_info.get("IsColorCam", False):
                if wb_r is not None:
                    self.camera.set_control_value(asi.ASI_WB_R, wb_r)
                    self.master_wb_r = wb_r
                if wb_b is not None:
                    self.camera.set_control_value(asi.ASI_WB_B, wb_b)
                    self.master_wb_b = wb_b

            # Store master parameters
            self.master_exposure_us = exposure_us
            self.master_gain = gain

            # Also update current hardware settings tracker
            self.current_exposure_us = exposure_us
            self.current_gain = gain

            # Use RGB24 for color camera, RAW8 for mono
            if self.camera_info.get("IsColorCam", False):
                self.camera.set_image_type(asi.ASI_IMG_RGB24)
                width = self.camera_info["MaxWidth"]
                height = self.camera_info["MaxHeight"]

                # Capture image directly
                img_buffer = self.camera.capture()
                img_array = np.frombuffer(img_buffer, dtype=np.uint8).reshape(
                    (height, width, 3)
                )

                # Convert to PIL Image
                pil_image = Image.fromarray(img_array)

            else:
                # Monochrome fallback
                self.camera.set_image_type(asi.ASI_IMG_RAW8)
                width = self.camera_info["MaxWidth"]
                height = self.camera_info["MaxHeight"]

                img_buffer = self.camera.capture()
                img_array = np.frombuffer(img_buffer, dtype=np.uint8).reshape(
                    (height, width)
                )

                # Convert to PIL Image
                pil_image = Image.fromarray(img_array, mode="L")

            # Encode to JPEG in memory - NO DISK WRITE
            buffer = io.BytesIO()
            pil_image.save(buffer, format="JPEG", quality=85)
            buffer.seek(0)

            return buffer.getvalue()

        except Exception as e:
            print(f"Fast capture failed: {e}")
            return None

    def capture_fast_observer(self) -> Optional[bytes]:
        """Observer capture: use master's parameters for consistent behavior.

        Returns:
            JPEG image as bytes using master's parameters, or None if capture failed
        """
        if not self.is_initialized or not self.camera:
            print("Camera not initialized")
            return None

        try:
            # Always use master parameters to ensure consistency
            # Only apply if different from current hardware settings to avoid unnecessary changes
            if (
                self.current_exposure_us != self.master_exposure_us
                or self.current_gain != self.master_gain
            ):
                self.camera.set_control_value(asi.ASI_EXPOSURE, self.master_exposure_us)
                self.camera.set_control_value(asi.ASI_GAIN, self.master_gain)
                self.current_exposure_us = self.master_exposure_us
                self.current_gain = self.master_gain

            # Always apply master white balance for color cameras
            if self.camera_info.get("IsColorCam", False):
                self.camera.set_control_value(asi.ASI_WB_R, self.master_wb_r)
                self.camera.set_control_value(asi.ASI_WB_B, self.master_wb_b)

            # Use RGB24 for color camera, RAW8 for mono
            if self.camera_info.get("IsColorCam", False):
                self.camera.set_image_type(asi.ASI_IMG_RGB24)
                width = self.camera_info["MaxWidth"]
                height = self.camera_info["MaxHeight"]

                # Capture image directly
                img_buffer = self.camera.capture()
                img_array = np.frombuffer(img_buffer, dtype=np.uint8).reshape(
                    (height, width, 3)
                )

                # Convert to PIL Image
                pil_image = Image.fromarray(img_array)

            else:
                # Monochrome fallback
                self.camera.set_image_type(asi.ASI_IMG_RAW8)
                width = self.camera_info["MaxWidth"]
                height = self.camera_info["MaxHeight"]

                img_buffer = self.camera.capture()
                img_array = np.frombuffer(img_buffer, dtype=np.uint8).reshape(
                    (height, width)
                )

                # Convert to PIL Image
                pil_image = Image.fromarray(img_array, mode="L")

            # Encode to JPEG in memory - NO DISK WRITE
            buffer = io.BytesIO()
            pil_image.save(buffer, format="JPEG", quality=85)
            buffer.seek(0)

            return buffer.getvalue()

        except Exception as e:
            print(f"Observer capture failed: {e}")
            return None

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
                "master_exposure_us": self.master_exposure_us,
                "master_gain": self.master_gain,
                "master_wb_r": self.master_wb_r,
                "master_wb_b": self.master_wb_b,
                "current_wb_r": current_settings.get("WB_R", 80)
                if self.camera_info.get("IsColorCam", False)
                else None,
                "current_wb_b": current_settings.get("WB_B", 100)
                if self.camera_info.get("IsColorCam", False)
                else None,
            }

        except Exception as e:
            return {"status": "error", "error": str(e)}

    def disconnect(self) -> None:
        """Disconnect from the camera."""
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
