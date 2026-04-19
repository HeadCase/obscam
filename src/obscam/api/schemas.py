"""Pydantic request and response models for the API layer."""

from typing import Any

from pydantic import BaseModel, ConfigDict


class SnapshotRequest(BaseModel):
    """Snapshot save request payload."""

    subdirectory: str | None = None
    filename_prefix: str | None = None


class LifecycleCommandResponse(BaseModel):
    """Backend lifecycle command response payload."""

    backend_state: str
    detail: str
    timestamp: float


class BackendStatusSection(BaseModel):
    """Backend lifecycle and recovery state."""

    state: str
    last_error: str | None = None
    last_transition_at: float
    consecutive_capture_failures: int
    last_recovery_attempt_at: float | None = None


class CameraStatusSection(BaseModel):
    """Connected camera state."""

    connected: bool
    status: str
    model: str | None = None


class CaptureStatusSection(BaseModel):
    """Latest frame and capture liveness state."""

    continuous_capture: bool
    has_frame: bool
    frame_timestamp: float | None = None
    frame_age_seconds: float | None = None
    frame_is_live: bool


class SettingsStatusSection(BaseModel):
    """Current or last known settings state."""

    model_config = ConfigDict(arbitrary_types_allowed=True)

    current_settings: dict[str, Any]


class BackendStatusResponse(BaseModel):
    """Structured backend status response."""

    backend: BackendStatusSection
    camera: CameraStatusSection
    capture: CaptureStatusSection
    settings: SettingsStatusSection
    timestamp: float
