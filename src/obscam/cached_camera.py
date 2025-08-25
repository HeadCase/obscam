#!/usr/bin/env python3
"""Cached camera wrapper for persistent settings and frame caching."""

import json
import os
import threading
import time
from pathlib import Path
from typing import Any

from obscam.camera_interface import CameraInterface
from obscam.constants import DEFAULT_CACHE_DIR


class CachedCamera:
    """Wrapper that adds persistent settings and frame caching to any camera
    implementation."""

    def __init__(self, camera: CameraInterface, cache_dir: Path = DEFAULT_CACHE_DIR):
        """Initialize cached camera wrapper.

        Args:
            camera: The underlying camera implementation
            cache_dir: Directory for caching (should be tmpfs for Pi)
        """
        self.camera: CameraInterface = camera
        self.cache_dir: Path = cache_dir
        self.settings_file: Path = cache_dir / "obscam_settings.json"
        self.frame_cache_file: Path = cache_dir / "obscam_latest_frame.jpg"

        # Thread safety
        self.settings_lock: threading.Lock = threading.Lock()
        self.cache_lock: threading.Lock = threading.Lock()

        # Continuous capture orchestration (moved from camera implementations)
        self.capture_thread: threading.Thread | None = None
        self.capture_running = False
        self.latest_frame: bytes | None = None
        self.frame_metadata: dict[str, Any] | None = None
        self.frame_lock = threading.Lock()

        cache_dir.mkdir(parents=True, exist_ok=True)

        # Load persisted settings
        self._load_settings()

    def _load_settings(self) -> None:
        """Load settings from cache file."""
        try:
            with self.settings_lock:
                if self.settings_file.exists():
                    with open(self.settings_file, "r") as f:
                        cached_settings = json.load(f)

                    self._cached_settings = cached_settings
                    print(f"Loaded cached settings: {cached_settings}")
                else:
                    self._cached_settings = None
                    print("No cached settings found, will use camera defaults")
        except Exception as e:
            print(f"Failed to load cached settings: {e}")
            self._cached_settings = None

    def _save_settings(self, settings: dict[str, Any]) -> None:
        """Atomically save settings to cache file."""
        try:
            with self.settings_lock:
                temp_file = f"{self.settings_file}.tmp"
                with open(temp_file, "w") as f:
                    json.dump(settings, f, indent=2)
                os.rename(temp_file, self.settings_file)  # Atomic on Unix
                print(f"Settings cached: {settings}")
        except Exception as e:
            print(f"Failed to cache settings: {e}")

    def _cache_frame(self, frame_bytes: bytes) -> None:
        """Cache latest frame to disk."""
        try:
            with self.cache_lock:
                temp_file = f"{self.frame_cache_file}.tmp"
                with open(temp_file, "wb") as f:
                    f.write(frame_bytes)
                os.rename(temp_file, self.frame_cache_file)  # Atomic
        except Exception as e:
            print(f"Failed to cache frame: {e}")

    def _load_cached_frame(self) -> bytes | None:
        """Load cached frame from disk."""
        try:
            with self.cache_lock:
                if os.path.exists(self.frame_cache_file):
                    with open(self.frame_cache_file, "rb") as f:
                        return f.read()
        except Exception as e:
            print(f"Failed to load cached frame: {e}")
        return None

    def connect(self) -> bool:
        """Connect to camera and apply any cached settings."""
        success = self.camera.connect()

        # Apply cached settings after successful connection
        if success and self._cached_settings:
            print("Applying cached settings to camera...")
            self.camera.update_settings(**self._cached_settings)

        return success

    def disconnect(self) -> None:
        """Disconnect from camera and stop capture service."""
        self.stop_continuous_capture()
        self.camera.disconnect()

    def capture_frame(self) -> bytes | None:
        """Delegate single frame capture to underlying camera."""
        return self.camera.capture_frame()

    def get_status(self) -> dict[str, Any]:
        """Get current camera status including service layer status."""
        status = self.camera.get_status()
        status["continuous_capture"] = self.capture_running
        return status

    def start_continuous_capture(self) -> bool:
        """Start continuous capture orchestration."""
        if self.camera.get_status().get("status") != "connected":
            print("Camera not connected - cannot start continuous capture")
            return False

        if self.capture_running:
            print("Continuous capture already running")
            return True

        try:
            self.capture_running = True
            self.capture_thread = threading.Thread(
                target=self._capture_loop,
                daemon=True,
                name="CachedCameraCaptureLoop",
            )
            self.capture_thread.start()
            print("Continuous capture started")
            return True
        except Exception as e:
            print(f"Failed to start continuous capture: {e}")
            self.capture_running = False
            return False

    def stop_continuous_capture(self) -> None:
        """Stop continuous capture orchestration."""
        if self.capture_running:
            self.capture_running = False
            if self.capture_thread:
                self.capture_thread.join(timeout=2.0)
            print("Continuous capture stopped")

    def _capture_loop(self) -> None:
        """Continuous capture loop orchestration."""
        print("Capture loop started")

        while self.capture_running:
            try:
                # Capture single frame from camera hardware
                frame_bytes = self.camera.capture_frame()

                if frame_bytes:
                    capture_time = time.time()
                    settings = self.camera.get_current_settings()

                    # Update in-memory cache
                    with self.frame_lock:
                        self.latest_frame = frame_bytes
                        self.frame_metadata = {**settings, "timestamp": capture_time}

                    # Cache to disk
                    self._cache_frame(frame_bytes)

                # Brief pause between captures
                time.sleep(0.1)

            except Exception as e:
                print(f"Capture loop error: {e}")
                time.sleep(1.0)

        print("Capture loop ended")

    def get_latest_frame(self) -> bytes | None:
        """Get latest frame from memory cache, with disk fallback."""
        # Try memory cache first
        with self.frame_lock:
            if self.latest_frame:
                return self.latest_frame

        # Fallback to cached frame from disk for new clients
        cached_frame = self._load_cached_frame()
        if cached_frame:
            print("Serving cached frame from disk")
        return cached_frame

    def get_frame_metadata(self) -> dict[str, Any] | None:
        """Get frame metadata from memory cache."""
        with self.frame_lock:
            return self.frame_metadata

    def update_settings(self, **settings: Any) -> bool:
        """Update camera settings and persist them."""
        success = self.camera.update_settings(**settings)

        if success:
            # Get the actual applied settings from camera
            current_settings = self.camera.get_current_settings()
            # Cache them for persistence
            self._save_settings(current_settings)

        return success

    def get_current_settings(self) -> dict[str, Any]:
        """Get current camera settings."""
        return self.camera.get_current_settings()
