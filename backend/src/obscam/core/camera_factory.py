"""Camera factory for backend service initialization."""

import os
from pathlib import Path

from obscam.core.backend_service import CameraBackendService
from obscam.common.logging_config import get_logger

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

    Returns:
        Backend service instance that manages camera operations
    """
    global _backend_service

    if _backend_service is None:
        cache_dir = _get_cache_directory()
        logger.info("Using ZWO ASI camera implementation")
        from obscam.camera.zwo_asi_camera import ZwoAsiCamera

        base_camera = ZwoAsiCamera()

        _backend_service = CameraBackendService(base_camera, cache_dir)
        logger.info("Backend service created", cache_dir=str(cache_dir))

    return _backend_service
