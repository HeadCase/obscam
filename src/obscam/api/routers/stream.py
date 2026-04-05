"""Streaming routes for MJPEG and telemetry delivery."""

import json
from typing import Annotated

from fastapi import APIRouter, Depends
from fastapi.responses import StreamingResponse

from obscam.api.dependencies import get_backend, get_runtime
from obscam.api.runtime import ApiRuntimeState
from obscam.core.backend_service import CameraBackendService

router = APIRouter()

BackendDep = Annotated[CameraBackendService, Depends(get_backend)]
RuntimeDep = Annotated[ApiRuntimeState, Depends(get_runtime)]


@router.get("/stream.mjpg")
async def mjpeg_stream(
    backend: BackendDep,
    runtime: RuntimeDep,
) -> StreamingResponse:
    """MJPEG video stream for efficient frame delivery."""
    runtime.bind_notifier_to_current_loop()
    boundary = "frame"

    async def generate_stream():
        last_generation = 0

        while True:
            snapshot = backend.frame_buffer.get_latest_snapshot()
            if snapshot is None or snapshot.generation <= last_generation:
                await runtime.delivery_notifier.wait_for_frame(last_generation)
                continue

            last_generation = snapshot.generation
            yield (
                (
                    f"--{boundary}\r\n"
                    "Content-Type: image/jpeg\r\n"
                    f"Content-Length: {len(snapshot.frame_bytes)}\r\n\r\n"
                ).encode("ascii")
                + snapshot.frame_bytes
                + b"\r\n"
            )

    return StreamingResponse(
        generate_stream(),
        media_type=f"multipart/x-mixed-replace; boundary={boundary}",
        headers={"Cache-Control": "no-store, max-age=0"},
    )


@router.get("/api/telemetry")
async def telemetry_sse(
    backend: BackendDep,
    runtime: RuntimeDep,
) -> StreamingResponse:
    """Server-Sent Events stream for telemetry data."""
    runtime.bind_notifier_to_current_loop()

    async def generate_telemetry():
        snapshot = backend.frame_buffer.get_latest_snapshot()
        settings = backend.get_current_settings() or {}
        initial_payload = runtime.build_telemetry_payload(
            snapshot,
            settings,
            include_settings_version=False,
        )
        last_generation = snapshot.generation if snapshot is not None else 0
        last_settings_version = runtime.settings_version

        yield f"event: snapshot\ndata: {json.dumps(initial_payload)}\n\n"

        while True:
            snapshot = backend.frame_buffer.get_latest_snapshot()
            settings = backend.get_current_settings() or {}
            current_generation = snapshot.generation if snapshot is not None else 0

            if (
                current_generation <= last_generation
                and runtime.settings_version <= last_settings_version
            ):
                await runtime.delivery_notifier.wait_for_update(
                    last_generation,
                    last_settings_version,
                )
                continue

            payload = runtime.build_telemetry_payload(
                snapshot,
                settings,
                include_settings_version=True,
            )
            last_generation = current_generation
            last_settings_version = runtime.settings_version
            yield f"event: frame\ndata: {json.dumps(payload)}\n\n"

    return StreamingResponse(
        generate_telemetry(),
        media_type="text/event-stream",
        headers={"Cache-Control": "no-cache", "Connection": "keep-alive"},
    )
