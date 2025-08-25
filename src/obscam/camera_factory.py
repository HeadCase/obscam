"""Camera factory for backend service initialization."""

import os
from pathlib import Path

from obscam.backend_service import CameraBackendService
from obscam.logging_config import get_logger

logger = get_logger("camera_factory")

_backend_service: CameraBackendService | None = None


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


def get_backend_service() -> CameraBackendService:
    """Get camera backend service (singleton).

    Uses OBSCAM_CAMERA_TYPE environment variable to select implementation:
    - 'gphoto2': Use libgphoto2 camera (for local development with Nikon Zf)
    - 'zwo' or unset: Use ZWO ASI camera (default for production on Pi)

    Returns:
        Backend service instance that manages camera operations
    """
    global _backend_service

    if _backend_service is None:
        camera_type = os.getenv("OBSCAM_CAMERA_TYPE", "zwo").lower()
        cache_dir = _get_cache_directory()

        if camera_type == "gphoto2":
            logger.info("Using gphoto2 camera implementation (development mode)")
            from obscam.libgphoto2_camera import Gphoto2Camera

            base_camera = Gphoto2Camera()
        else:
            logger.info("Using ZWO ASI camera implementation (production mode)")
            from obscam.zwo_asi_camera import ZwoAsiCamera

            base_camera = ZwoAsiCamera()

        _backend_service = CameraBackendService(base_camera, cache_dir)
        logger.info("Backend service created", cache_dir=str(cache_dir))

    return _backend_service
