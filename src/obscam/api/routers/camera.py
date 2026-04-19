"""Camera control, backend lifecycle, and snapshot routes."""

import time
from datetime import datetime
from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException, Request
from fastapi.responses import Response

from obscam.api.dependencies import get_backend, get_runtime
from obscam.api.runtime import ApiRuntimeState
from obscam.api.schemas import LifecycleCommandResponse, SnapshotRequest
from obscam.common.constants import ASSETS_DIR
from obscam.common.logging_config import get_logger
from obscam.core.backend_service import (
    BackendLifecycleState,
    BackendStatusSnapshot,
    CameraBackendService,
)
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


@router.get("/api/backend/status")
async def get_backend_status(backend: BackendDep) -> BackendStatusSnapshot:
    """Get current structured backend and camera status."""
    try:
        return backend.get_status_snapshot()
    except Exception as exc:
        logger.error("Failed to get backend status", error=str(exc))
        raise HTTPException(
            status_code=500,
            detail=f"Failed to get backend status: {exc}",
        ) from exc


@router.post("/api/backend/start")
async def start_backend(backend: BackendDep) -> dict[str, object]:
    """Start the backend service if it is currently stopped."""
    initial_state = backend.get_backend_state()
    if initial_state != BackendLifecycleState.STOPPED.value:
        return LifecycleCommandResponse(
            backend_state=initial_state,
            detail="Backend already active",
            timestamp=time.time(),
        ).model_dump()

    if not backend.start_backend():
        raise HTTPException(status_code=503, detail="Failed to start backend service")

    logger.info("Backend service started via API")
    return LifecycleCommandResponse(
        backend_state=backend.get_backend_state(),
        detail="Backend started",
        timestamp=time.time(),
    ).model_dump()


@router.post("/api/backend/recover")
async def recover_backend(backend: BackendDep) -> dict[str, object]:
    """Request an immediate recovery attempt when the backend is unhealthy."""
    state = backend.get_backend_state()
    if state == BackendLifecycleState.RUNNING.value:
        return LifecycleCommandResponse(
            backend_state=state,
            detail="Backend already running",
            timestamp=time.time(),
        ).model_dump()

    if state in {
        BackendLifecycleState.STOPPED.value,
        BackendLifecycleState.STARTING.value,
        BackendLifecycleState.STOPPING.value,
    }:
        raise HTTPException(
            status_code=409,
            detail=f"Cannot recover backend while {state}",
        )

    if not backend.request_recovery():
        raise HTTPException(status_code=409, detail="Recovery request rejected")

    return LifecycleCommandResponse(
        backend_state=backend.get_backend_state(),
        detail="Recovery requested",
        timestamp=time.time(),
    ).model_dump()


@router.post("/api/backend/stop")
async def stop_backend(backend: BackendDep) -> dict[str, object]:
    """Stop the backend service."""
    if backend.get_backend_state() == BackendLifecycleState.STOPPED.value:
        return LifecycleCommandResponse(
            backend_state=BackendLifecycleState.STOPPED.value,
            detail="Backend already stopped",
            timestamp=time.time(),
        ).model_dump()

    backend.stop_backend()
    return LifecycleCommandResponse(
        backend_state=backend.get_backend_state(),
        detail="Backend stopped",
        timestamp=time.time(),
    ).model_dump()


@router.get("/api/latest-frame")
async def get_latest_frame(backend: BackendDep) -> Response:
    """Get the latest frame from the in-memory buffer, including stale frames."""
    try:
        frame_data = backend.get_latest_frame_with_metadata()
        if frame_data is None:
            raise HTTPException(status_code=503, detail="No frame available")

        frame_bytes, frame_metadata = frame_data
        current_time = time.time()
        frame_timestamp = frame_metadata.get("timestamp", 0)
        frame_age_seconds = current_time - frame_timestamp if frame_timestamp > 0 else 0
        status = backend.get_status_snapshot()
        frame_is_live = status["capture"]["frame_is_live"]

        return Response(
            content=frame_bytes,
            media_type="image/jpeg",
            headers={
                "Cache-Control": "no-cache, no-store, must-revalidate",
                "Pragma": "no-cache",
                "Expires": "0",
                "X-Frame-Timestamp": str(frame_timestamp),
                "X-Frame-Age-Seconds": str(round(frame_age_seconds, 2)),
                "X-Frame-Is-Live": str(frame_is_live).lower(),
            },
        )
    except HTTPException:
        raise
    except Exception as exc:
        logger.error("Frame retrieval failed", error=str(exc))
        raise HTTPException(
            status_code=500,
            detail=f"Frame retrieval failed: {exc}",
        ) from exc


@router.post("/api/snapshots")
async def create_snapshot(
    payload: SnapshotRequest,
    backend: BackendDep,
) -> dict[str, object]:
    """Persist the latest buffered frame to disk."""
    try:
        if backend.get_backend_state() == BackendLifecycleState.STOPPED.value:
            raise HTTPException(status_code=503, detail="Backend service not running")

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
            status_code=500,
            detail=f"Snapshot save failed: {exc}",
        ) from exc


@router.get("/api/frame-info")
async def get_frame_info(backend: BackendDep) -> dict[str, object]:
    """Get information about the latest frame and backend status."""
    try:
        return {
            "has_frame": backend.get_frame_metadata() is not None,
            "frame_metadata": backend.get_frame_metadata(),
            "current_settings": backend.get_current_settings(),
            "backend_status": backend.get_status_snapshot(),
            "timestamp": time.time(),
        }
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
        status = backend.get_status_snapshot()
        return {
            "current_settings": status["settings"]["current_settings"],
            "status": status,
            "capabilities": backend.get_control_capabilities(),
            "stream_url": "/stream.mjpg",
            "telemetry_url": "/api/telemetry",
            "timestamp": time.time(),
        }
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
        if backend.get_backend_state() != BackendLifecycleState.RUNNING.value:
            raise HTTPException(status_code=400, detail="Backend service not running")

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
            status_code=500,
            detail=f"Settings update failed: {exc}",
        ) from exc
