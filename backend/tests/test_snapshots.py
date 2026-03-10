import asyncio
from pathlib import Path

from fastapi import HTTPException

from obscam.api import main as api_main
from obscam.core.backend_service import CameraBackendService


class DummyCamera:
    def connect(self) -> bool:
        return True

    def disconnect(self) -> None:
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


class FakeBackend:
    def __init__(
        self,
        started: bool = True,
        frame_data: tuple[bytes, dict[str, object]] | None = None,
        start_result: bool = True,
    ):
        self.started = started
        self.frame_data = frame_data
        self.start_result = start_result

    def is_started(self) -> bool:
        return self.started

    def start_backend(self) -> bool:
        if self.start_result:
            self.started = True
            return True
        return False

    def get_latest_frame_with_metadata(self) -> tuple[bytes, dict[str, object]] | None:
        return self.frame_data

    def get_current_settings(self) -> dict[str, object]:
        return {"exposure_ms": 200.0, "gain": 250}


def test_get_latest_frame_with_metadata_returns_none_when_buffer_empty(tmp_path: Path):
    service = CameraBackendService(DummyCamera(), tmp_path)
    service._started = True

    assert service.get_latest_frame_with_metadata() is None


def test_resolve_snapshot_directory_accepts_relative_subdirectory(tmp_path: Path, monkeypatch):
    monkeypatch.setattr(api_main, "ASSETS_DIR", tmp_path / "assets")

    resolved = api_main._resolve_snapshot_directory("interesting/frames")

    assert resolved == (tmp_path / "assets" / "interesting" / "frames").resolve()


def test_resolve_snapshot_directory_rejects_invalid_paths(tmp_path: Path, monkeypatch):
    monkeypatch.setattr(api_main, "ASSETS_DIR", tmp_path / "assets")

    for invalid in ("../foo", "/tmp/foo", "foo/../../bar"):
        try:
            api_main._resolve_snapshot_directory(invalid)
        except Exception as exc:
            assert getattr(exc, "status_code", None) == 400
        else:
            raise AssertionError(f"Expected invalid path to fail: {invalid}")


def test_sanitize_filename_prefix():
    assert api_main._sanitize_filename_prefix("  Roof Event!!  ") == "roof-event"
    assert api_main._sanitize_filename_prefix("___") == "snapshot"
    assert api_main._sanitize_filename_prefix(None) == "snapshot"


def test_snapshot_endpoint_saves_file_and_returns_metadata(tmp_path: Path, monkeypatch):
    frame_data = (
        b"jpeg-bytes",
        {"timestamp": 1234.5, "exposure_ms": 200.0, "gain": 250},
    )
    fake_backend = FakeBackend(started=True, frame_data=frame_data)

    monkeypatch.setattr(api_main, "backend", fake_backend)
    monkeypatch.setattr(api_main, "ASSETS_DIR", tmp_path / "assets")

    payload = asyncio.run(
        api_main.create_snapshot(
            api_main.SnapshotRequest(
                subdirectory="interesting-roof-events",
                filename_prefix="Roof Event",
            )
        )
    )

    saved_path = tmp_path / payload["relative_path"]
    assert saved_path.exists()
    assert saved_path.read_bytes() == b"jpeg-bytes"
    assert payload["filename"].startswith("roof-event_")
    assert payload["relative_path"].startswith("assets/interesting-roof-events/")
    assert payload["current_settings"] == {"exposure_ms": 200.0, "gain": 250}


def test_snapshot_endpoint_returns_503_when_backend_unavailable(monkeypatch):
    fake_backend = FakeBackend(started=False, frame_data=None, start_result=False)

    monkeypatch.setattr(api_main, "backend", fake_backend)

    try:
        asyncio.run(api_main.create_snapshot(api_main.SnapshotRequest()))
    except HTTPException as exc:
        assert exc.status_code == 503
        assert exc.detail == "Backend service not available"
    else:
        raise AssertionError("Expected backend-unavailable snapshot to fail")


def test_snapshot_endpoint_returns_503_when_no_frame_available(tmp_path: Path, monkeypatch):
    fake_backend = FakeBackend(started=True, frame_data=None)

    monkeypatch.setattr(api_main, "backend", fake_backend)
    monkeypatch.setattr(api_main, "ASSETS_DIR", tmp_path / "assets")

    try:
        asyncio.run(api_main.create_snapshot(api_main.SnapshotRequest()))
    except HTTPException as exc:
        assert exc.status_code == 503
        assert exc.detail == "No frame available"
    else:
        raise AssertionError("Expected no-frame snapshot to fail")


def test_snapshot_endpoint_creates_assets_subdirectories(tmp_path: Path, monkeypatch):
    frame_data = (b"jpeg-bytes", {"timestamp": 1234.5})
    fake_backend = FakeBackend(started=True, frame_data=frame_data)

    monkeypatch.setattr(api_main, "backend", fake_backend)
    monkeypatch.setattr(api_main, "ASSETS_DIR", tmp_path / "assets")

    payload = asyncio.run(
        api_main.create_snapshot(api_main.SnapshotRequest(subdirectory="nested/folder"))
    )

    assert (tmp_path / payload["relative_path"]).exists()
