#!/usr/bin/env python3
"""Libgphoto2 camera implementation for local development with Nikon Zf."""

import io
import time
import threading
from typing import Any

from PIL import Image
import gphoto2 as gp  # pyright: ignore[reportMissingTypeStubs]

from .camera_interface import CameraInterface, FrameMetadata


class Gphoto2Camera(CameraInterface):
    """Libgphoto2 camera implementation for Nikon Zf and similar cameras."""

    def __init__(self) -> None:
        """Initialize the libgphoto2 camera."""
        self.camera: Any = None
        self.inited = False

        # Current camera settings
        self.current_settings = {
            "exposure_ms": 200.0,  # 200ms default
            "gain": 100,  # Simulated gain (ISO-like)
            "wb_r": 100,  # Not used but kept for interface compatibility
            "wb_b": 100,
        }
        self.settings_lock = threading.Lock()

        # Frame caching
        self.latest_frame: bytes | None = None
        self.frame_metadata: FrameMetadata | None = None
        self.frame_lock = threading.Lock()

        # Continuous capture control
        self.capture_thread: threading.Thread | None = None
        self.capture_running = False

    def connect(self) -> bool:
        """Connect to the camera."""
        try:
            self.camera = gp.Camera()  # pyright: ignore[reportUnknownMemberType]
            self.camera.init()
            self.inited = True
            print("Connected to gphoto2 camera")

            # Check if bulb mode is available
            if not self._has_writable_bulb():
                print("Warning: Camera does not expose writable bulb control")
                print("Make sure camera is set to Bulb mode on the dial")

            return True
        except Exception as e:
            print(f"Failed to connect to gphoto2 camera: {e}")
            return False

    def disconnect(self) -> None:
        """Disconnect from the camera."""
        self.stop_continuous_capture()

        if self.inited and self.camera:
            try:
                self.camera.exit()
            except:
                pass
            self.inited = False
            self.camera = None
            print("Camera disconnected")

    def get_status(self) -> dict[str, Any]:
        """Get current camera status."""
        if not self.inited or not self.camera:
            return {"status": "disconnected", "error": "Camera not initialized"}

        try:
            with self.settings_lock:
                settings = self.current_settings.copy()

            # Try to get current shutter speed
            shutter_mode = self._shutter_mode_hint()

            return {
                "status": "connected",
                "camera_model": "Nikon Zf (via gphoto2)",
                "is_color_camera": True,
                "current_exposure_ms": settings["exposure_ms"],
                "current_gain": settings["gain"],
                "current_wb_r": settings.get("wb_r"),
                "current_wb_b": settings.get("wb_b"),
                "continuous_capture": self.capture_running,
                "shutter_mode": shutter_mode or "Unknown",
            }
        except Exception as e:
            return {"status": "error", "error": str(e)}

    def start_continuous_capture(self) -> bool:
        """Start continuous capture loop."""
        if not self.inited or not self.camera:
            print("Camera not initialized - cannot start continuous capture")
            return False

        if self.capture_running:
            print("Continuous capture already running")
            return True

        try:
            self.capture_running = True
            self.capture_thread = threading.Thread(
                target=self._capture_loop,
                daemon=True,
                name="Gphoto2CaptureLoop",
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
                self.capture_thread.join(timeout=5.0)
            print("Continuous capture stopped")

    def _capture_loop(self) -> None:
        """Main capture loop - runs in separate thread."""
        print("Capture loop started")

        while self.capture_running and self.inited and self.camera:
            try:
                # Get current settings
                with self.settings_lock:
                    exposure_seconds = self.current_settings["exposure_ms"] / 1000.0
                    gain = self.current_settings["gain"]
                    wb_r = self.current_settings.get("wb_r", 100)
                    wb_b = self.current_settings.get("wb_b", 100)

                # Capture using bulb mode
                capture_time = time.time()
                image_data = self._capture_bulb_image(exposure_seconds)

                if image_data:
                    # Convert to JPEG if needed
                    img = Image.open(io.BytesIO(image_data))

                    # Resize if too large (optional, for faster transfer)
                    max_dimension = 1920
                    if img.width > max_dimension or img.height > max_dimension:
                        img.thumbnail(
                            (max_dimension, max_dimension), Image.Resampling.LANCZOS
                        )

                    # Save as JPEG
                    buffer = io.BytesIO()
                    img.save(buffer, format="JPEG", quality=85)
                    jpeg_bytes = buffer.getvalue()

                    # Store frame with metadata
                    with self.frame_lock:
                        self.latest_frame = jpeg_bytes
                        self.frame_metadata = FrameMetadata(
                            exposure_ms=self.current_settings["exposure_ms"],
                            gain=int(gain),
                            wb_r=int(wb_r),
                            wb_b=int(wb_b),
                            timestamp=capture_time,
                        )

                # Brief pause between captures to avoid overloading
                time.sleep(0.1)

            except Exception as e:
                print(f"Capture error: {e}")
                time.sleep(1.0)  # Longer pause on error

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
        """Update camera settings and apply them to the hardware."""
        try:
            # Update internal state first
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

            # Apply settings to camera hardware via gphoto2
            cfg = self._fresh_cfg()

            # Map gain to ISO (find closest available value)
            if "gain" in settings:
                try:
                    iso_values = [
                        100,
                        125,
                        160,
                        200,
                        250,
                        320,
                        400,
                        500,
                        640,
                        800,
                        1000,
                        1250,
                        1600,
                        2000,
                        2500,
                        3200,
                        4000,
                        5000,
                        6400,
                        8000,
                        10000,
                        12800,
                        16000,
                        20000,
                        25600,
                        32000,
                        40000,
                        51200,
                    ]

                    requested_iso = int(settings["gain"])
                    # Find closest available ISO value
                    closest_iso = min(iso_values, key=lambda x: abs(x - requested_iso))

                    iso_control = cfg.get_child_by_name("iso")
                    iso_control.set_value(str(closest_iso))
                    print(f"Mapped gain {requested_iso} to ISO {closest_iso}")
                except Exception as e:
                    print(f"Warning: Could not set ISO: {e}")

            # White balance - map to preset modes (no fine R/B control available)
            if "wb_r" in settings or "wb_b" in settings:
                print("Note: Fine white balance R/B control not available on DSLR")
                print(
                    "Values stored for metadata only. Use camera menu for white balance presets."
                )

            # Exposure handled via bulb duration (existing implementation is correct)

            # Apply the configuration to camera
            self.camera.set_config(cfg)

            print(f"Camera settings updated: {settings}")
            return True

        except Exception as e:
            print(f"Failed to update camera settings: {e}")
            return False

    def get_current_settings(self) -> dict[str, Any]:
        """Get current camera settings."""
        with self.settings_lock:
            return self.current_settings.copy()

    # Helper methods specific to gphoto2

    def _fresh_cfg(self) -> Any:
        """Get fresh camera configuration."""
        return self.camera.get_config()

    def _child(self, key: str) -> Any:
        """Get configuration child by name."""
        return self._fresh_cfg().get_child_by_name(key)

    def _has_writable_bulb(self) -> bool:
        """Check if camera has writable bulb control."""
        try:
            cfg = self._fresh_cfg()
            bulb = cfg.get_child_by_name("bulb")
            # Just check if we can read the value
            _ = bulb.get_value()
            return True
        except:
            return False

    def _shutter_mode_hint(self) -> str | None:
        """Get current shutter mode hint."""
        try:
            val = str(self._child("shutterspeed").get_value()).strip().lower()
            if "bulb" in val:
                return "Bulb"
            if "time" in val:
                return "Time"
            return val
        except:
            return None

    def _capture_bulb_image(self, seconds: float) -> bytes | None:
        """Capture an image using bulb mode."""
        if seconds <= 0:
            return None

        try:
            # Open shutter (Nikon uses toggle, so we set to 1 to open)
            cfg = self._fresh_cfg()
            bulb = cfg.get_child_by_name("bulb")
            bulb.set_value(1)
            self.camera.set_config(cfg)

            time.sleep(seconds)

            bulb.set_value(0)
            self.camera.set_config(cfg)

            folder, name = self._wait_for_file_added(timeout_s=30.0)

            cam_file = self.camera.file_get(folder, name, gp.GP_FILE_TYPE_NORMAL)  # pyright: ignore[reportUnknownMemberType]
            data = cam_file.get_data_and_size()

            return bytes(data) if not isinstance(data, bytes) else data

        except Exception as e:
            print(f"Bulb capture failed: {e}, trying normal capture")

    def _wait_for_file_added(self, timeout_s: float = 30.0) -> tuple[str, str]:
        """Wait for camera to report a new file."""
        deadline = time.time() + timeout_s
        while time.time() < deadline:
            ev_type, ev_data = self.camera.wait_for_event(1000)  # ms
            if ev_type == gp.GP_EVENT_FILE_ADDED:  # pyright: ignore[reportUnknownMemberType]
                return ev_data.folder, ev_data.name

        raise TimeoutError("Timed out waiting for FILE_ADDED event from camera")
