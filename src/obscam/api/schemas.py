"""Pydantic request models for the API layer."""

from pydantic import BaseModel


class SnapshotRequest(BaseModel):
    """Snapshot save request payload."""

    subdirectory: str | None = None
    filename_prefix: str | None = None
