"""FastAPI dependency helpers for the web layer."""

from typing import cast

from fastapi import Request

from obscam.api.runtime import ApiRuntimeState
from obscam.core.backend_service import CameraBackendService


def get_runtime(request: Request) -> ApiRuntimeState:
    """Return the shared API runtime state for the current app."""
    return cast(ApiRuntimeState, request.app.state.runtime)


def get_backend(request: Request) -> CameraBackendService:
    """Return the lazily initialized camera backend."""
    runtime = get_runtime(request)
    return runtime.ensure_backend()
