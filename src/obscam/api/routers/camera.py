"""Camera control and snapshot routes."""

import time
from datetime import datetime
from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException, Request
from fastapi.responses import Response

from obscam.api.dependencies import get_backend, get_runtime
from obscam.api.runtime import ApiRuntimeState
from obscam.api.schemas import SnapshotRequest
from obscam.common.constants import ASSETS_DIR
from obscam.common.logging_config import get_logger
from obscam.core.backend_service import CameraBackendService
from obscam.core.settings_service import validate_settings_payload
from obscam.core.snapshot_service import (
    SnapshotPathError,
    build_relative_asset_path,
    build_snapshot_filename,
    ensure_unique_snapshot_path,
    resolve_snapshot_directory,
    sanitize_filename_prefix,
)

router = APIRouter()
logger = get_logger("api_camera")

BackendDep = Annotated[CameraBackendService, Depends(get_backend)]
RuntimeDep = Annotated[ApiRuntimeState, Depends(get_runtime)]


@router.get("/api/status")
async def get_status(backend: BackendDep) -> dict[str, object]:
    """Get current backend and camera status."""
    try:
        backend_status = backend.get_status()
        return {**backend_status, "timestamp": time.time()}
    except Exception as exc:
        logger.error("Failed to get backend status", error=str(exc))
        return {
            "status": "error",
            "message": f"Failed to get backend status: {exc}",
            "timestamp": time.time(),
        }


@router.get("/api/connect")
async def connect_camera(backend: BackendDep) -> dict[str, object]:
    """Start the backend service."""
    try:
        if backend.start_backend():
            logger.info("Backend service started via API")
            return {
                "status": "connected",
                "message": "Backend service started successfully",
                "timestamp": time.time(),
            }

        return {
            "status": "error",
            "message": "Failed to start backend service",
            "timestamp": time.time(),
        }
    except Exception as exc:
        logger.error("Backend startup error", error=str(exc))
        return {
            "status": "error",
            "message": f"Backend startup error: {exc}",
            "timestamp": time.time(),
        }


@router.get("/api/latest-frame")
async def get_latest_frame(backend: BackendDep) -> Response:
    """Get the latest frame from the backend service."""
    try:
        if not backend.is_started():
            raise HTTPException(status_code=400, detail="Backend service not started")

        frame_data = backend.get_latest_frame_with_metadata()
        if frame_data is None:
            raise HTTPException(status_code=503, detail="No frame available")

        frame_bytes, frame_metadata = frame_data
        current_time = time.time()
        frame_timestamp = frame_metadata.get("timestamp", 0)
        frame_age_seconds = current_time - frame_timestamp if frame_timestamp > 0 else 0

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
    except HTTPException:
        raise
    except Exception as exc:
        logger.error("Frame retrieval failed", error=str(exc))
        raise HTTPException(
            status_code=500, detail=f"Frame retrieval failed: {exc}"
        ) from exc


@router.post("/api/snapshots")
async def create_snapshot(
    payload: SnapshotRequest,
    backend: BackendDep,
) -> dict[str, object]:
    """Persist the latest buffered frame to disk."""
    try:
        if not backend.is_started() and not backend.start_backend():
            raise HTTPException(status_code=503, detail="Backend service not available")

        frame_data = backend.get_latest_frame_with_metadata()
        if frame_data is None:
            raise HTTPException(status_code=503, detail="No frame available")

        frame_bytes, frame_metadata = frame_data
        try:
            target_dir = resolve_snapshot_directory(ASSETS_DIR, payload.subdirectory)
        except SnapshotPathError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

        target_dir.mkdir(parents=True, exist_ok=True)
        filename_prefix = sanitize_filename_prefix(payload.filename_prefix)
        snapshot_path = ensure_unique_snapshot_path(
            target_dir,
            build_snapshot_filename(filename_prefix, datetime.now()),
        )
        snapshot_path.write_bytes(frame_bytes)

        saved_at = time.time()
        frame_timestamp = float(frame_metadata.get("timestamp", 0.0) or 0.0)
        frame_age_seconds = saved_at - frame_timestamp if frame_timestamp > 0 else 0.0
        current_settings = backend.get_current_settings()

        logger.info(
            "Snapshot saved",
            path=str(snapshot_path),
            frame_age_seconds=round(frame_age_seconds, 3),
            current_settings=current_settings,
        )

        return {
            "status": "success",
            "filename": snapshot_path.name,
            "relative_path": build_relative_asset_path(ASSETS_DIR, snapshot_path),
            "saved_at": saved_at,
            "frame_timestamp": frame_timestamp,
            "frame_age_seconds": round(frame_age_seconds, 2),
            "current_settings": current_settings,
        }
    except HTTPException:
        raise
    except Exception as exc:
        logger.error("Snapshot save failed", error=str(exc))
        raise HTTPException(
            status_code=500, detail=f"Snapshot save failed: {exc}"
        ) from exc


@router.get("/api/frame-info")
async def get_frame_info(backend: BackendDep) -> dict[str, object]:
    """Get information about the latest frame and backend status."""
    try:
        if not backend.is_started():
            raise HTTPException(status_code=400, detail="Backend service not started")

        return {
            "has_frame": backend.get_frame_metadata() is not None,
            "frame_metadata": backend.get_frame_metadata(),
            "current_settings": backend.get_current_settings(),
            "backend_status": backend.get_status(),
            "timestamp": time.time(),
        }
    except HTTPException:
        raise
    except Exception as exc:
        logger.error("Frame info failed", error=str(exc))
        raise HTTPException(
            status_code=500,
            detail=f"Frame info failed: {exc}",
        ) from exc


@router.get("/api/bootstrap")
async def bootstrap(backend: BackendDep) -> dict[str, object]:
    """Bootstrap endpoint for initial page load and camera capabilities."""
    try:
        if not backend.is_started() and not backend.start_backend():
            raise HTTPException(status_code=400, detail="Backend service not available")

        return {
            "current_settings": backend.get_current_settings(),
            "status": backend.get_status(),
            "capabilities": backend.get_control_capabilities(),
            "stream_url": "/stream.mjpg",
            "telemetry_url": "/api/telemetry",
            "timestamp": time.time(),
        }
    except HTTPException:
        raise
    except Exception as exc:
        logger.error("Bootstrap failed", error=str(exc))
        raise HTTPException(status_code=500, detail=f"Bootstrap failed: {exc}") from exc


@router.post("/api/settings")
async def update_settings(
    request: Request,
    backend: BackendDep,
    runtime: RuntimeDep,
) -> dict[str, object]:
    """Queue validated settings updates and notify stream listeners."""
    try:
        if not backend.is_started():
            raise HTTPException(status_code=400, detail="Backend service not started")

        data = await request.json()
        valid_settings = validate_settings_payload(
            data,
            backend.get_control_capabilities(),
        )
        if not valid_settings:
            raise HTTPException(status_code=400, detail="No valid settings provided")

        backend.queue_settings_update(valid_settings)
        applied_version = runtime.publish_settings_update()

        return {
            "status": "success",
            "applied_version": applied_version,
            "current_settings": backend.get_current_settings(),
            "timestamp": time.time(),
        }
    except HTTPException:
        raise
    except Exception as exc:
        logger.error("Settings update failed", error=str(exc))
        raise HTTPException(
            status_code=500, detail=f"Settings update failed: {exc}"
        ) from exc
