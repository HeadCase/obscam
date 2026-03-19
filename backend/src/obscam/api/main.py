"""Main FastAPI application entry point."""

import asyncio
import json
import re
import time
from collections import deque
from datetime import datetime
from pathlib import Path, PurePosixPath

import uvicorn
from fastapi import FastAPI, HTTPException, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import Response, StreamingResponse
from fastapi.templating import Jinja2Templates
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel

from obscam.common.constants import ASSETS_DIR, PROJECT_ROOT
from obscam.common.logging_config import get_logger
from obscam.core.backend_service import CameraBackendService
from obscam.core.camera_factory import get_backend_service

logger = get_logger("api_main")

static_dir = PROJECT_ROOT / "frontend/static"
template_dir = PROJECT_ROOT / "frontend/templates"
templates = Jinja2Templates(directory=str(template_dir))

# Create FastAPI app
app = FastAPI(title="ObsCam API", version="1.0.0")

app.mount("/static", StaticFiles(directory=str(static_dir)), name="static")

# Setup CORS middleware
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# Backend service - will be initialized in start_server after logging is configured

backend: CameraBackendService = None  # type: ignore # Will be initialized in start_server()

DEFAULT_EXPOSURE_RANGE_MS = (0.032, 30000.0)
DEFAULT_GAIN_RANGE = (0, 600)


class SnapshotRequest(BaseModel):
    """Snapshot save request payload."""

    subdirectory: str | None = None
    filename_prefix: str | None = None


def _capability_range(
    capabilities: dict[str, object], key: str, default_min: float, default_max: float
) -> tuple[float, float]:
    cap = capabilities.get(key)
    if isinstance(cap, dict):
        min_val = cap.get("min")
        max_val = cap.get("max")
        if isinstance(min_val, (int, float)) and isinstance(max_val, (int, float)):
            return float(min_val), float(max_val)
    return float(default_min), float(default_max)


# Global state for MJPEG streaming and SSE
new_frame_event = asyncio.Event()
settings_version = 0
settings_applied_event = asyncio.Event()
frame_times = deque(maxlen=120)

# No custom shutdown handling - let FastAPI/Python handle Ctrl+C immediately


# Wire frame notification callback
def _on_new_frame():
    """Called when a new frame is available."""
    global frame_times
    new_frame_event.set()
    new_frame_event.clear()  # Clear for next wait
    frame_times.append(time.time())


def _ensure_backend() -> CameraBackendService:
    """Initialize the backend singleton when routes are used outside
    start_server()."""
    global backend

    if backend is None:
        backend = get_backend_service()
        backend.frame_buffer.on_new_frame = _on_new_frame

    return backend


def _resolve_snapshot_directory(subdirectory: str | None) -> Path:
    """Resolve an optional snapshot subdirectory under the assets root."""
    assets_root = ASSETS_DIR.resolve()
    assets_root.mkdir(parents=True, exist_ok=True)

    if subdirectory is None or not subdirectory.strip():
        return assets_root

    normalized = PurePosixPath(subdirectory.strip())
    if normalized.is_absolute():
        raise HTTPException(
            status_code=400, detail="Snapshot subdirectory must be relative"
        )

    parts = normalized.parts
    if not parts or any(part in {"", ".", ".."} for part in parts):
        raise HTTPException(status_code=400, detail="Invalid snapshot subdirectory")

    target_dir = (assets_root / Path(*parts)).resolve()
    try:
        target_dir.relative_to(assets_root)
    except ValueError as exc:
        raise HTTPException(
            status_code=400, detail="Snapshot subdirectory must stay under assets/"
        ) from exc

    return target_dir


def _sanitize_filename_prefix(filename_prefix: str | None) -> str:
    """Sanitize user-supplied filename prefixes to a narrow safe subset."""
    candidate = (filename_prefix or "snapshot").strip().lower()
    candidate = re.sub(r"[^a-z0-9_-]+", "-", candidate)
    candidate = re.sub(r"[-_]+", "-", candidate).strip("-_")
    return candidate or "snapshot"


def _build_snapshot_filename(filename_prefix: str, current_time: datetime) -> str:
    """Build a sortable JPEG snapshot filename with millisecond precision."""
    timestamp = current_time.strftime("%Y%m%d_%H%M%S")
    milliseconds = current_time.microsecond // 1000
    return f"{filename_prefix}_{timestamp}_{milliseconds:03d}.jpg"


def _ensure_unique_snapshot_path(directory: Path, filename: str) -> Path:
    """Resolve filename collisions by appending a numeric suffix."""
    snapshot_path = directory / filename
    if not snapshot_path.exists():
        return snapshot_path

    stem = snapshot_path.stem
    suffix = snapshot_path.suffix
    counter = 1
    while True:
        candidate = directory / f"{stem}_{counter}{suffix}"
        if not candidate.exists():
            return candidate
        counter += 1


# Frame buffer callback will be connected in start_server after backend is initialized


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


@app.post("/api/snapshots")
async def create_snapshot(payload: SnapshotRequest):
    """Persist the latest buffered frame to disk."""
    active_backend = _ensure_backend()

    try:
        if not active_backend.is_started() and not active_backend.start_backend():
            raise HTTPException(status_code=503, detail="Backend service not available")

        frame_data = active_backend.get_latest_frame_with_metadata()
        if frame_data is None:
            raise HTTPException(status_code=503, detail="No frame available")

        frame_bytes, frame_metadata = frame_data
        target_dir = _resolve_snapshot_directory(payload.subdirectory)
        target_dir.mkdir(parents=True, exist_ok=True)

        filename_prefix = _sanitize_filename_prefix(payload.filename_prefix)
        snapshot_path = _ensure_unique_snapshot_path(
            target_dir,
            _build_snapshot_filename(filename_prefix, datetime.now()),
        )
        snapshot_path.write_bytes(frame_bytes)

        saved_at = time.time()
        frame_timestamp = float(frame_metadata.get("timestamp", 0.0) or 0.0)
        frame_age_seconds = saved_at - frame_timestamp if frame_timestamp > 0 else 0.0
        relative_path = (
            Path("assets") / snapshot_path.relative_to(ASSETS_DIR.resolve())
        ).as_posix()
        current_settings = active_backend.get_current_settings()

        logger.info(
            "Snapshot saved",
            path=str(snapshot_path),
            frame_age_seconds=round(frame_age_seconds, 3),
            current_settings=current_settings,
        )

        return {
            "status": "success",
            "filename": snapshot_path.name,
            "relative_path": relative_path,
            "saved_at": saved_at,
            "frame_timestamp": frame_timestamp,
            "frame_age_seconds": round(frame_age_seconds, 2),
            "current_settings": current_settings,
        }
    except HTTPException:
        raise
    except Exception as e:
        logger.error("Snapshot save failed", error=str(e))
        raise HTTPException(status_code=500, detail=f"Snapshot save failed: {e}")


@app.post("/api/update-settings")
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
                    detail=f"Exposure must be between {exposure_min}ms and {exposure_max}ms",
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
            # Wait for new frame
            await new_frame_event.wait()

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
            # Wait for new frame or settings change
            done, pending = await asyncio.wait(
                [
                    asyncio.create_task(new_frame_event.wait()),
                    asyncio.create_task(settings_applied_event.wait()),
                ],
                return_when=asyncio.FIRST_COMPLETED,
            )

            # Cancel pending task
            for task in pending:
                task.cancel()

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

        # Use the capture loop's coalescing for real-time updates
        valid_settings = {}
        if "exposure_ms" in data:
            exposure_ms = float(data["exposure_ms"])
            if exposure_min <= exposure_ms <= exposure_max:
                valid_settings["exposure_ms"] = exposure_ms

        if "gain" in data:
            gain = int(data["gain"])
            if gain_min <= gain <= gain_max:
                valid_settings["gain"] = gain

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
    """Run the FastAPI server with proper shutdown handling."""
    # Simple uvicorn configuration
    config = uvicorn.Config(
        app,
        host="0.0.0.0",
        port=port,
        log_level="info",
        timeout_keep_alive=2,  # Shorter keepalive for faster shutdown
    )
    server = uvicorn.Server(config)

    try:
        server.run()
    except KeyboardInterrupt:
        logger.info("Server interrupted by user")


def start_server():
    """Start the web server with backend service and graceful shutdown."""
    global backend

    # Initialize backend AFTER logging is configured
    if backend is None:
        backend = get_backend_service()
        # Connect frame buffer callback
        backend.frame_buffer.on_new_frame = _on_new_frame

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
    logger.info("Press Ctrl+C to stop")

    try:
        run_server()
    finally:
        # Graceful shutdown
        logger.info("Shutting down backend service...")
        backend.shutdown_gracefully()
        logger.info("Shutdown complete. Goodbye!")


if __name__ == "__main__":
    start_server()
