"""Snapshot file naming and path resolution helpers."""

import re
from datetime import datetime
from pathlib import Path, PurePosixPath


class SnapshotPathError(ValueError):
    """Raised when a snapshot path is invalid."""


def resolve_snapshot_directory(assets_dir: Path, subdirectory: str | None) -> Path:
    """Resolve an optional snapshot subdirectory under the assets root."""
    assets_root = assets_dir.resolve()
    assets_root.mkdir(parents=True, exist_ok=True)

    if subdirectory is None or not subdirectory.strip():
        return assets_root

    normalized = PurePosixPath(subdirectory.strip())
    if normalized.is_absolute():
        raise SnapshotPathError("Snapshot subdirectory must be relative")

    parts = normalized.parts
    if not parts or any(part in {"", ".", ".."} for part in parts):
        raise SnapshotPathError("Invalid snapshot subdirectory")

    target_dir = (assets_root / Path(*parts)).resolve()
    try:
        target_dir.relative_to(assets_root)
    except ValueError as exc:
        raise SnapshotPathError(
            "Snapshot subdirectory must stay under assets/"
        ) from exc

    return target_dir


def sanitize_filename_prefix(filename_prefix: str | None) -> str:
    """Sanitize user-supplied filename prefixes to a narrow safe subset."""
    candidate = (filename_prefix or "snapshot").strip().lower()
    candidate = re.sub(r"[^a-z0-9_-]+", "-", candidate)
    candidate = re.sub(r"[-_]+", "-", candidate).strip("-_")
    return candidate or "snapshot"


def build_snapshot_filename(filename_prefix: str, current_time: datetime) -> str:
    """Build a sortable JPEG snapshot filename with millisecond precision."""
    timestamp = current_time.strftime("%Y%m%d_%H%M%S")
    milliseconds = current_time.microsecond // 1000
    return f"{filename_prefix}_{timestamp}_{milliseconds:03d}.jpg"


def ensure_unique_snapshot_path(directory: Path, filename: str) -> Path:
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


def build_relative_asset_path(assets_dir: Path, snapshot_path: Path) -> str:
    """Build the public relative asset path for a saved snapshot."""
    return (Path("assets") / snapshot_path.relative_to(assets_dir.resolve())).as_posix()
