"""FastAPI routes for obscam camera backend."""

import time
from typing import cast

from fastapi import HTTPException, Request
from fastapi.responses import Response

from obscam.common.logging_config import get_logger
from obscam.core.camera_factory import get_backend_service

logger = get_logger("api_routes")

# Initialize backend service
backend = get_backend_service()

DEFAULT_EXPOSURE_RANGE_MS = (0.032, 30000.0)
DEFAULT_GAIN_RANGE = (0, 600)


def _capability_range(
    capabilities: dict[str, object], key: str, default_min: float, default_max: float
) -> tuple[float, float]:
    cap = capabilities.get(key)
    if isinstance(cap, dict):
        typed_cap = cast(dict[str, object], cap)
        min_val = typed_cap.get("min")
        max_val = typed_cap.get("max")
        if isinstance(min_val, (int, float)) and isinstance(max_val, (int, float)):
            return float(min_val), float(max_val)
    return float(default_min), float(default_max)


async def get_status():
    """Get current backend and camera status."""
    try:
        backend_status = backend.get_status()

        return {
            **backend_status,
            "timestamp": time.time(),
        }
    except Exception as e:
        logger.error("Failed to get backend status", error=str(e))
        return {
            "status": "error",
            "message": f"Failed to get backend status: {e}",
            "timestamp": time.time(),
        }


async def connect_camera():
    """Start the backend service (connects camera and starts capture)."""
    try:
        if backend.start_backend():
            logger.info("Backend service started via API")
            return {
                "status": "connected",
                "message": "Backend service started successfully",
                "timestamp": time.time(),
            }
        else:
            return {
                "status": "error",
                "message": "Failed to start backend service",
                "timestamp": time.time(),
            }
    except Exception as e:
        logger.error("Backend startup error", error=str(e))
        return {
            "status": "error",
            "message": f"Backend startup error: {e}",
            "timestamp": time.time(),
        }


async def get_latest_frame():
    """Get the latest frame from backend service."""
    try:
        if not backend.is_started():
            raise HTTPException(status_code=400, detail="Backend service not started")

        frame_bytes = backend.get_latest_frame()
        frame_metadata = backend.get_frame_metadata()

        if frame_bytes:
            # Calculate frame age for honest timestamp reporting
            current_time = time.time()
            frame_timestamp = (
                frame_metadata.get("timestamp", 0) if frame_metadata else 0
            )
            frame_age_seconds = (
                current_time - frame_timestamp if frame_timestamp > 0 else 0
            )

            return Response(
                content=frame_bytes,
                media_type="image/jpeg",
                headers={
                    "Cache-Control": "no-cache, no-store, must-revalidate",
                    "Pragma": "no-cache",
                    "Expires": "0",
                    "X-Frame-Timestamp": str(frame_timestamp),
                    "X-Frame-Age-Seconds": str(round(frame_age_seconds, 2)),
                },
            )
        else:
            raise HTTPException(status_code=503, detail="No frame available")

    except HTTPException:
        raise
    except Exception as e:
        logger.error("Frame retrieval failed", error=str(e))
        raise HTTPException(
            status_code=500, detail=f"Frame retrieval failed: {e}"
        ) from e


async def update_settings(request: Request):
    """Update camera settings (stateless, no session required)."""
    try:
        if not backend.is_started():
            raise HTTPException(status_code=400, detail="Backend service not started")

        data = await request.json()
        capabilities = backend.camera.get_control_capabilities()
        exposure_min, exposure_max = _capability_range(
            capabilities,
            "exposure_ms",
            DEFAULT_EXPOSURE_RANGE_MS[0],
            DEFAULT_EXPOSURE_RANGE_MS[1],
        )
        gain_min, gain_max = _capability_range(
            capabilities, "gain", DEFAULT_GAIN_RANGE[0], DEFAULT_GAIN_RANGE[1]
        )

        # Validate and sanitize settings
        valid_settings = {}
        if "exposure_ms" in data:
            exposure_ms = float(data["exposure_ms"])
            if exposure_min <= exposure_ms <= exposure_max:
                valid_settings["exposure_ms"] = exposure_ms
            else:
                raise HTTPException(
                    status_code=400,
                    detail=(
                        f"Exposure must be between {exposure_min}ms "
                        f"and {exposure_max}ms"
                    ),
                )

        if "gain" in data:
            gain = int(data["gain"])
            if gain_min <= gain <= gain_max:
                valid_settings["gain"] = gain
            else:
                raise HTTPException(
                    status_code=400,
                    detail=f"Gain must be between {gain_min} and {gain_max}",
                )

        if not valid_settings:
            raise HTTPException(status_code=400, detail="No valid settings provided")

        success = backend.update_settings(**valid_settings)

        if success:
            current_settings = backend.get_current_settings()
            return {
                "status": "success",
                "message": "Settings updated",
                "current_settings": current_settings,
                "timestamp": time.time(),
            }
        else:
            raise HTTPException(
                status_code=500, detail="Failed to update camera settings"
            )

    except HTTPException:
        raise
    except Exception as e:
        raise HTTPException(
            status_code=500, detail=f"Settings update failed: {e}"
        ) from e


async def get_frame_info():
    """Get information about the latest frame and backend status."""
    try:
        if not backend.is_started():
            raise HTTPException(status_code=400, detail="Backend service not started")

        frame_metadata = backend.get_frame_metadata()
        current_settings = backend.get_current_settings()
        status = backend.get_status()

        return {
            "has_frame": frame_metadata is not None,
            "frame_metadata": frame_metadata,
            "current_settings": current_settings,
            "backend_status": status,
            "timestamp": time.time(),
        }

    except Exception as e:
        logger.error("Frame info failed", error=str(e))
        raise HTTPException(status_code=500, detail=f"Frame info failed: {e}") from e
