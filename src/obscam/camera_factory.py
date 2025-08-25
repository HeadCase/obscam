"""Camera factory for selecting the appropriate camera implementation."""

import os
from pathlib import Path

from obscam.cached_camera import CachedCamera
from obscam.camera_interface import CameraInterface


_camera_instance: CameraInterface | None = None


def _get_cache_directory() -> Path:
    """Get appropriate cache directory for the platform."""
    if os.path.exists("/dev/shm"):
        return Path("/dev/shm")
    elif os.path.exists("/tmp"):
        cache_dir = Path("/tmp/obscam")
        cache_dir.mkdir(parents=True, exist_ok=True)
        return cache_dir
    else:
        return Path("/tmp")


def get_camera() -> CameraInterface:
    """Get the appropriate camera instance based on environment.

    Uses OBSCAM_CAMERA_TYPE environment variable to select implementation:
    - 'gphoto2': Use libgphoto2 camera (for local development with Nikon Zf)
    - 'zwo' or unset: Use ZWO ASI camera (default for production on Pi)

    Returns:
        Camera instance implementing CameraInterface
    """
    global _camera_instance

    if _camera_instance is None:
        camera_type = os.getenv("OBSCAM_CAMERA_TYPE", "zwo").lower()

        if camera_type == "gphoto2":
            print("Using gphoto2 camera implementation (development mode)")
            from .libgphoto2_camera import Gphoto2Camera

            base_camera = Gphoto2Camera()
        else:
            # Default to ZWO for production
            print("Using ZWO ASI camera implementation (production mode)")
            from .zwo_asi_camera import ZwoAsiCamera

            base_camera = ZwoAsiCamera()

        cache_dir = _get_cache_directory()

        _camera_instance = CachedCamera(base_camera, cache_dir)
        print(f"Camera caching enabled in: {cache_dir}")

    return _camera_instance
