#!/usr/bin/env python3
"""Camera factory for selecting the appropriate camera implementation."""

import os

from .camera_interface import CameraInterface


# Global camera instance
_camera_instance: CameraInterface | None = None


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

            _camera_instance = Gphoto2Camera()
        else:
            # Default to ZWO for production
            print("Using ZWO ASI camera implementation (production mode)")
            from .zwo_asi_camera import ZwoAsiCamera

            _camera_instance = ZwoAsiCamera()

    return _camera_instance
