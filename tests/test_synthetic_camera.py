from pathlib import Path

from PIL import Image

from obscam.camera.synthetic_camera import SyntheticCamera
from obscam.core import camera_factory


def _write_fixture(path: Path, color: int) -> None:
    image = Image.new("L", (160, 90), color=color)
    image.save(path, format="JPEG")


def test_synthetic_camera_emits_jpeg_frames_from_fixture_directory(
    tmp_path: Path, monkeypatch
):
    fixture_dir = tmp_path / "fixtures"
    fixture_dir.mkdir()
    _write_fixture(fixture_dir / "frame_01.jpg", 30)
    _write_fixture(fixture_dir / "frame_02.jpg", 190)

    camera = SyntheticCamera(frame_dir=fixture_dir)
    monkeypatch.setattr("obscam.camera.synthetic_camera.time.sleep", lambda _: None)

    assert camera.connect() is True

    frame_bytes = camera.capture_frame()

    assert frame_bytes is not None
    assert frame_bytes.startswith(b"\xff\xd8")


def test_synthetic_camera_consecutive_frames_change(tmp_path: Path, monkeypatch):
    fixture_dir = tmp_path / "fixtures"
    fixture_dir.mkdir()
    _write_fixture(fixture_dir / "frame_01.jpg", 30)
    _write_fixture(fixture_dir / "frame_02.jpg", 190)

    camera = SyntheticCamera(frame_dir=fixture_dir)
    monkeypatch.setattr("obscam.camera.synthetic_camera.time.sleep", lambda _: None)
    assert camera.connect() is True

    first_frame = camera.capture_frame()
    second_frame = camera.capture_frame()

    assert first_frame != second_frame


def test_synthetic_camera_gain_changes_output(tmp_path: Path, monkeypatch):
    fixture_dir = tmp_path / "fixtures"
    fixture_dir.mkdir()
    _write_fixture(fixture_dir / "frame_01.jpg", 80)

    camera = SyntheticCamera(frame_dir=fixture_dir)
    monkeypatch.setattr("obscam.camera.synthetic_camera.time.sleep", lambda _: None)
    assert camera.connect() is True

    low_gain_frame = camera.capture_frame()
    assert camera.update_settings(gain=600) is True
    high_gain_frame = camera.capture_frame()

    assert low_gain_frame != high_gain_frame


def test_camera_factory_selects_synthetic_backend(tmp_path: Path, monkeypatch):
    fixture_dir = tmp_path / "fixtures"
    fixture_dir.mkdir()
    _write_fixture(fixture_dir / "frame_01.jpg", 40)

    monkeypatch.setenv("OBSCAM_CAMERA_BACKEND", "synthetic")
    monkeypatch.setenv("OBSCAM_SYNTHETIC_FRAME_DIR", str(fixture_dir))
    monkeypatch.setattr(camera_factory, "_backend_service", None)

    backend_service = camera_factory.get_backend_service()

    assert isinstance(backend_service.camera, SyntheticCamera)

    backend_service.stop_backend()
    monkeypatch.setattr(camera_factory, "_backend_service", None)


def test_synthetic_camera_reports_expected_capabilities(tmp_path: Path):
    fixture_dir = tmp_path / "fixtures"
    fixture_dir.mkdir()
    _write_fixture(fixture_dir / "frame_01.jpg", 120)

    camera = SyntheticCamera(frame_dir=fixture_dir)
    capabilities = camera.get_control_capabilities()

    assert capabilities["camera_type"] == "Synthetic Camera"
    assert capabilities["exposure_ms"]["max"] == 30000.0
    assert capabilities["gain"]["max"] == 600
