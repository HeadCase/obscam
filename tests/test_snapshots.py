import asyncio
from pathlib import Path
from typing import TypedDict, cast

from fastapi import HTTPException

from obscam.api.routers import camera as camera_routes
from obscam.api.schemas import SnapshotRequest
from obscam.core.backend_service import BackendLifecycleState, CameraBackendService
from obscam.core.snapshot_service import (
    resolve_snapshot_directory,
    sanitize_filename_prefix,
)


class DummyCamera:
    def connect(self) -> bool:
        return True

    def disconnect(self) -> None:
        return None

    def interrupt_capture(self) -> None:
        return None

    def get_status(self) -> dict[str, object]:
        return {"status": "connected"}

    def capture_frame(self) -> bytes | None:
        return b"dummy-frame"

    def update_settings(self, **settings: object) -> bool:
        return True

    def get_current_settings(self) -> dict[str, object]:
        return {"exposure_ms": 200.0, "gain": 250}

    def get_control_capabilities(self) -> dict[str, object]:
        return {
            "exposure_ms": {"min": 0.032, "max": 30000.0, "type": "float"},
            "gain": {"min": 0, "max": 600, "type": "int"},
        }


class FakeFrameBuffer:
    def __init__(self) -> None:
        self.on_new_frame = None

    def get_latest_snapshot(self):
        return None


class FakeBackend:
    def __init__(
        self,
        *,
        state: str = BackendLifecycleState.RUNNING.value,
        frame_data: tuple[bytes, dict[str, object]] | None = None,
    ) -> None:
        self.state = state
        self.frame_data = frame_data
        self.frame_buffer = FakeFrameBuffer()

    def get_backend_state(self) -> str:
        return self.state

    def get_latest_frame_with_metadata(self) -> tuple[bytes, dict[str, object]] | None:
        return self.frame_data

    def get_current_settings(self) -> dict[str, object]:
        return {"exposure_ms": 200.0, "gain": 250}


class SnapshotPayload(TypedDict):
    filename: str
    relative_path: str
    current_settings: dict[str, object]


def _typed_backend(fake_backend: FakeBackend) -> CameraBackendService:
    return cast(CameraBackendService, fake_backend)


def test_get_latest_frame_with_metadata_returns_none_when_buffer_empty(tmp_path: Path):
    service = CameraBackendService(DummyCamera(), tmp_path)

    assert service.get_latest_frame_with_metadata() is None


def test_resolve_snapshot_directory_accepts_relative_subdirectory(
    tmp_path: Path,
) -> None:
    resolved = resolve_snapshot_directory(tmp_path / "assets", "interesting/frames")

    assert resolved == (tmp_path / "assets" / "interesting" / "frames").resolve()


def test_resolve_snapshot_directory_rejects_invalid_paths(tmp_path: Path) -> None:
    for invalid in ("../foo", "/tmp/foo", "foo/../../bar"):
        try:
            resolve_snapshot_directory(tmp_path / "assets", invalid)
        except Exception as exc:
            assert str(exc)
        else:
            raise AssertionError(f"Expected invalid path to fail: {invalid}")


def test_sanitize_filename_prefix() -> None:
    assert sanitize_filename_prefix("  Roof Event!!  ") == "roof-event"
    assert sanitize_filename_prefix("___") == "snapshot"
    assert sanitize_filename_prefix(None) == "snapshot"


def test_snapshot_endpoint_saves_file_and_returns_metadata(
    tmp_path: Path, monkeypatch
) -> None:
    frame_data = cast(
        tuple[bytes, dict[str, object]],
        (
            b"jpeg-bytes",
            {"timestamp": 1234.5, "exposure_ms": 200.0, "gain": 250},
        ),
    )
    fake_backend = FakeBackend(
        state=BackendLifecycleState.RUNNING.value,
        frame_data=frame_data,
    )
    monkeypatch.setattr(camera_routes, "ASSETS_DIR", tmp_path / "assets")

    payload = cast(
        SnapshotPayload,
        asyncio.run(
            camera_routes.create_snapshot(
                SnapshotRequest(
                    subdirectory="interesting-roof-events",
                    filename_prefix="Roof Event",
                ),
                _typed_backend(fake_backend),
            )
        ),
    )

    saved_path = tmp_path / payload["relative_path"]
    assert saved_path.exists()
    assert saved_path.read_bytes() == b"jpeg-bytes"
    assert payload["filename"].startswith("roof-event_")
    assert payload["relative_path"].startswith("assets/interesting-roof-events/")
    assert payload["current_settings"] == {"exposure_ms": 200.0, "gain": 250}


def test_snapshot_endpoint_returns_503_when_backend_stopped() -> None:
    fake_backend = FakeBackend(
        state=BackendLifecycleState.STOPPED.value,
        frame_data=None,
    )

    try:
        asyncio.run(
            camera_routes.create_snapshot(
                SnapshotRequest(),
                _typed_backend(fake_backend),
            )
        )
    except HTTPException as exc:
        assert exc.status_code == 503
        assert exc.detail == "Backend service not running"
    else:
        raise AssertionError("Expected stopped-backend snapshot to fail")


def test_snapshot_endpoint_returns_503_when_no_frame_available(
    tmp_path: Path, monkeypatch
) -> None:
    fake_backend = FakeBackend(
        state=BackendLifecycleState.RUNNING.value,
        frame_data=None,
    )
    monkeypatch.setattr(camera_routes, "ASSETS_DIR", tmp_path / "assets")

    try:
        asyncio.run(
            camera_routes.create_snapshot(
                SnapshotRequest(),
                _typed_backend(fake_backend),
            )
        )
    except HTTPException as exc:
        assert exc.status_code == 503
        assert exc.detail == "No frame available"
    else:
        raise AssertionError("Expected no-frame snapshot to fail")


def test_snapshot_endpoint_creates_assets_subdirectories(
    tmp_path: Path, monkeypatch
) -> None:
    frame_data = cast(
        tuple[bytes, dict[str, object]], (b"jpeg-bytes", {"timestamp": 1234.5})
    )
    fake_backend = FakeBackend(
        state=BackendLifecycleState.RUNNING.value,
        frame_data=frame_data,
    )
    monkeypatch.setattr(camera_routes, "ASSETS_DIR", tmp_path / "assets")

    payload = cast(
        SnapshotPayload,
        asyncio.run(
            camera_routes.create_snapshot(
                SnapshotRequest(subdirectory="nested/folder"),
                _typed_backend(fake_backend),
            )
        ),
    )

    assert (tmp_path / payload["relative_path"]).exists()
