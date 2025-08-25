"""Main FastAPI application entry point."""

import time
import asyncio
import json
from collections import deque
import uvicorn
from fastapi import FastAPI, Request, HTTPException
from fastapi.responses import Response, StreamingResponse
from fastapi.templating import Jinja2Templates
from fastapi.middleware.cors import CORSMiddleware

from obscam.core.camera_factory import get_backend_service
from obscam.common.logging_config import get_logger
from obscam.common.constants import PROJECT_ROOT

logger = get_logger("api_main")

# Get template directory (relative to this file location)
template_dir = PROJECT_ROOT / "frontend/templates"
templates = Jinja2Templates(directory=str(template_dir))

# Create FastAPI app
app = FastAPI(title="ObsCam API", version="1.0.0")

# Setup CORS middleware
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# Initialize backend service
backend = get_backend_service()

# Global state for MJPEG streaming and SSE
new_frame_event = asyncio.Event()
settings_version = 0
settings_applied_event = asyncio.Event()
frame_times = deque(maxlen=120)


# Wire frame notification callback
def _on_new_frame():
    """Called when a new frame is available."""
    global frame_times
    new_frame_event.set()
    new_frame_event.clear()  # Clear for next wait
    frame_times.append(time.time())


# Connect frame buffer callback
backend.frame_buffer.on_new_frame = _on_new_frame


# Template routes (replacing Flask)
@app.get("/")
async def index(request: Request):
    """Main page displaying the camera feed."""
    return templates.TemplateResponse("index.html", {"request": request})


# API routes
@app.get("/api/status")
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


@app.get("/api/connect")
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


@app.get("/api/latest-frame")
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


@app.post("/api/update-settings")
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


@app.get("/api/frame-info")
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


@app.get("/stream.mjpg")
async def mjpeg_stream():
    """MJPEG video stream for efficient frame delivery."""
    boundary = "frame"

    async def generate_stream():
        while True:
            try:
                # Wait for new frame (or timeout for heartbeat)
                await asyncio.wait_for(new_frame_event.wait(), timeout=2.0)
            except asyncio.TimeoutError:
                # Send heartbeat to keep connection alive
                continue

            # Get latest frame
            frame_bytes = backend.get_latest_frame()
            if not frame_bytes:
                continue

            # Send MJPEG frame
            frame_data = (
                (
                    f"--{boundary}\r\n"
                    "Content-Type: image/jpeg\r\n"
                    f"Content-Length: {len(frame_bytes)}\r\n\r\n"
                ).encode("ascii")
                + frame_bytes
                + b"\r\n"
            )

            yield frame_data

    return StreamingResponse(
        generate_stream(),
        media_type=f"multipart/x-mixed-replace; boundary={boundary}",
        headers={"Cache-Control": "no-store, max-age=0"},
    )


def _estimate_fps(window_s: float = 5.0) -> float | None:
    """Estimate FPS from recent frame times."""
    if len(frame_times) < 2:
        return None
    now = time.time()
    recent = [t for t in frame_times if now - t <= window_s]
    if len(recent) < 2:
        return None
    return round((len(recent) - 1) / (recent[-1] - recent[0]), 1)


@app.get("/api/telemetry")
async def telemetry_sse():
    """Server-Sent Events stream for telemetry data."""

    async def generate_telemetry():
        # Send initial snapshot
        metadata = backend.get_frame_metadata() or {}
        settings = backend.get_current_settings() or {}
        fps = _estimate_fps()

        initial_payload = {
            "timestamp": metadata.get("timestamp"),
            "capture_ms": metadata.get("capture_duration_ms"),
            "has_frame": backend.frame_buffer.has_frame(),
            "fps": fps,
            "settings": settings,
        }

        yield f"event: snapshot\ndata: {json.dumps(initial_payload)}\n\n"

        while True:
            try:
                # Wait for new frame or settings change
                tasks = [
                    asyncio.create_task(
                        asyncio.wait_for(new_frame_event.wait(), timeout=5.0)
                    ),
                    asyncio.create_task(
                        asyncio.wait_for(settings_applied_event.wait(), timeout=5.0)
                    ),
                ]

                done, pending = await asyncio.wait(
                    tasks, return_when=asyncio.FIRST_COMPLETED
                )

                # Cancel pending tasks
                for task in pending:
                    task.cancel()

            except asyncio.TimeoutError:
                # Send periodic heartbeat
                status = backend.get_status()
                yield f"event: heartbeat\ndata: {json.dumps({'status': status.get('backend_service', 'unknown')})}\n\n"
                continue

            # Build telemetry payload
            metadata = backend.get_frame_metadata() or {}
            settings = backend.get_current_settings() or {}
            fps = _estimate_fps()

            payload = {
                "timestamp": metadata.get("timestamp"),
                "capture_ms": metadata.get("capture_duration_ms"),
                "has_frame": backend.frame_buffer.has_frame(),
                "fps": fps,
                "settings": settings,
                "settings_version": settings_version,
            }

            yield f"event: frame\ndata: {json.dumps(payload)}\n\n"

    return StreamingResponse(
        generate_telemetry(),
        media_type="text/event-stream",
        headers={"Cache-Control": "no-cache", "Connection": "keep-alive"},
    )


@app.get("/api/bootstrap")
async def bootstrap():
    """Bootstrap endpoint for initial page load - gets current settings and capabilities."""
    try:
        if not backend.is_started():
            # Try to start backend automatically
            if not backend.start_backend():
                raise HTTPException(
                    status_code=400, detail="Backend service not available"
                )

        current_settings = backend.get_current_settings()
        status = backend.get_status()
        capabilities = backend.camera.get_control_capabilities()

        return {
            "current_settings": current_settings,
            "status": status,
            "capabilities": capabilities,
            "stream_url": "/stream.mjpg",
            "telemetry_url": "/api/telemetry",
            "timestamp": time.time(),
        }

    except Exception as e:
        logger.error("Bootstrap failed", error=str(e))
        raise HTTPException(status_code=500, detail=f"Bootstrap failed: {e}")


@app.post("/api/settings")
async def update_settings_v2(request: Request):
    """Enhanced settings update with coalescing and version tracking."""
    global settings_version

    try:
        if not backend.is_started():
            raise HTTPException(status_code=400, detail="Backend service not started")

        data = await request.json()

        # Use the capture loop's coalescing for real-time updates
        valid_settings = {}
        if "exposure_ms" in data:
            exposure_ms = float(data["exposure_ms"])
            if 0.1 <= exposure_ms <= 30000:
                valid_settings["exposure_ms"] = exposure_ms

        if "gain" in data:
            gain = int(data["gain"])
            if 0 <= gain <= 51200:  # Support both camera types
                valid_settings["gain"] = gain

        if "wb_r" in data:
            wb_r = int(data["wb_r"])
            if 50 <= wb_r <= 150:
                valid_settings["wb_r"] = wb_r

        if "wb_b" in data:
            wb_b = int(data["wb_b"])
            if 50 <= wb_b <= 150:
                valid_settings["wb_b"] = wb_b

        if not valid_settings:
            raise HTTPException(status_code=400, detail="No valid settings provided")

        # Queue settings update for capture loop (coalescing)
        backend.capture_loop.update_settings(valid_settings)

        # Async persistence
        backend.settings_manager.save_settings_async(
            {**backend.get_current_settings(), **valid_settings}
        )

        # Notify SSE listeners
        settings_version += 1
        settings_applied_event.set()
        settings_applied_event.clear()

        return {
            "status": "success",
            "applied_version": settings_version,
            "current_settings": backend.get_current_settings(),
            "timestamp": time.time(),
        }

    except HTTPException:
        raise
    except Exception as e:
        logger.error("Settings update failed", error=str(e))
        raise HTTPException(status_code=500, detail=f"Settings update failed: {e}")


def run_server(port: int = 8000):
    """Run the FastAPI server."""
    uvicorn.run(app, host="0.0.0.0", port=port, log_level="info")


def start_server():
    """Start the web server with backend service."""
    logger.info("Starting backend service...")
    if backend.start_backend():
        logger.info("Backend service started successfully!")
        logger.info("Available endpoints:")
        logger.info("  - Main page: http://localhost:8000/")
        logger.info("  - Latest frame: http://localhost:8000/api/latest-frame")
        logger.info(
            "  - Update settings: POST http://localhost:8000/api/update-settings"
        )
        logger.info("  - Status: http://localhost:8000/api/status")
    else:
        logger.warning("Backend service failed to start. Use /api/connect to retry.")

    # Start the server
    logger.info("Starting web server on port 8000...")

    try:
        run_server()
    finally:
        # Graceful shutdown
        logger.info("Shutting down backend service...")
        backend.shutdown_gracefully()


if __name__ == "__main__":
    start_server()
