"""FastAPI routes for obscam camera backend."""

import time
from fastapi import HTTPException, Request
from fastapi.responses import Response

from obscam.core.camera_factory import get_backend_service
from obscam.common.logging_config import get_logger

logger = get_logger("api_routes")

# Initialize backend service
backend = get_backend_service()


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
        raise HTTPException(status_code=500, detail=f"Frame retrieval failed: {e}")


async def update_settings(request: Request):
    """Update camera settings (stateless, no session required)."""
    try:
        if not backend.is_started():
            raise HTTPException(status_code=400, detail="Backend service not started")

        data = await request.json()

        # Validate and sanitize settings
        valid_settings = {}
        if "exposure_ms" in data:
            exposure_ms = float(data["exposure_ms"])
            if 0.1 <= exposure_ms <= 30000:  # 0.1ms to 30s range
                valid_settings["exposure_ms"] = exposure_ms
            else:
                raise HTTPException(
                    status_code=400, detail="Exposure must be between 0.1ms and 30s"
                )

        if "gain" in data:
            gain = int(data["gain"])
            if 0 <= gain <= 51200:  # Extended range for both ZWO and DSLR
                valid_settings["gain"] = gain
            else:
                raise HTTPException(
                    status_code=400, detail="Gain must be between 0 and 51200"
                )

        if "wb_r" in data:
            wb_r = int(data["wb_r"])
            if 50 <= wb_r <= 150:  # White balance range
                valid_settings["wb_r"] = wb_r
            else:
                raise HTTPException(
                    status_code=400,
                    detail="Red white balance must be between 50 and 150",
                )

        if "wb_b" in data:
            wb_b = int(data["wb_b"])
            if 50 <= wb_b <= 150:  # White balance range
                valid_settings["wb_b"] = wb_b
            else:
                raise HTTPException(
                    status_code=400,
                    detail="Blue white balance must be between 50 and 150",
                )

        if "image_format" in data:
            image_format = str(data["image_format"]).lower()
            if image_format in ["mono", "color"]:
                valid_settings["image_format"] = image_format
            else:
                raise HTTPException(
                    status_code=400,
                    detail="Image format must be 'mono' or 'color'",
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
        raise HTTPException(status_code=500, detail=f"Settings update failed: {e}")


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
        raise HTTPException(status_code=500, detail=f"Frame info failed: {e}")
