import threading

import numpy as np
import zwoasi as asi  # pyright: ignore[reportMissingTypeStubs]

from obscam.camera.zwo_asi_camera import ZwoAsiCamera


class FakeAsiCamera:
    def __init__(self) -> None:
        self.calls: list[str] = []
        self.video_frame = np.zeros((2, 2), dtype=np.uint8)
        self.single_frame = np.ones((2, 2), dtype=np.uint8)

    def set_control_value(self, control: int, value: int) -> None:
        self.calls.append(f"set_control:{control}:{value}")

    def set_image_type(self, image_type: int) -> None:
        self.calls.append(f"set_image_type:{image_type}")

    def capture_video_frame(self, timeout=None):
        self.calls.append(f"capture_video_frame_timeout:{timeout}")
        self.calls.append("capture_video_frame")
        return self.video_frame

    def capture(self):
        self.calls.append("capture")
        return self.single_frame

    def start_video_capture(self) -> None:
        self.calls.append("start_video_capture")

    def stop_video_capture(self) -> None:
        self.calls.append("stop_video_capture")

    def stop_exposure(self) -> None:
        self.calls.append("stop_exposure")


def _build_camera(exposure_ms: float, fake_sdk_camera: FakeAsiCamera) -> ZwoAsiCamera:
    camera = object.__new__(ZwoAsiCamera)
    camera.camera_info = {"MaxWidth": 2, "MaxHeight": 2}
    camera.camera = fake_sdk_camera
    camera.is_initialized = True
    camera.settings_lock = threading.Lock()
    camera.current_capture_mode = ""
    camera.video_mode_threshold_ms = 200.0
    camera.current_settings = {"exposure_ms": exposure_ms, "gain": 300}
    camera._capture_count = 0
    camera._last_frame_signature = None
    camera._duplicate_frame_streak = 0
    return camera


def test_zwo_asi_camera_uses_video_mode_for_short_exposures() -> None:
    fake_sdk_camera = FakeAsiCamera()
    camera = _build_camera(50.0, fake_sdk_camera)

    frame = camera.capture_frame()

    assert frame is not None
    assert frame.startswith(b"\xff\xd8")
    assert fake_sdk_camera.calls.count("start_video_capture") == 1
    assert fake_sdk_camera.calls.count("capture_video_frame") == 1
    assert fake_sdk_camera.calls.count("capture") == 0
    assert camera.current_capture_mode == "video"


def test_zwo_asi_camera_reuses_video_mode_without_restart() -> None:
    fake_sdk_camera = FakeAsiCamera()
    camera = _build_camera(25.0, fake_sdk_camera)

    first_frame = camera.capture_frame()
    second_frame = camera.capture_frame()

    assert first_frame is not None
    assert second_frame is not None
    assert fake_sdk_camera.calls.count("start_video_capture") == 1
    assert fake_sdk_camera.calls.count("capture_video_frame") == 2


def test_zwo_asi_camera_switches_back_to_single_mode_for_long_exposures() -> None:
    fake_sdk_camera = FakeAsiCamera()
    camera = _build_camera(25.0, fake_sdk_camera)

    first_frame = camera.capture_frame()
    camera.current_settings["exposure_ms"] = 250.0
    second_frame = camera.capture_frame()

    assert first_frame is not None
    assert second_frame is not None
    assert fake_sdk_camera.calls.count("start_video_capture") == 1
    assert fake_sdk_camera.calls.count("stop_video_capture") == 1
    assert fake_sdk_camera.calls.count("capture") == 1


def test_zwo_asi_camera_uses_single_capture_for_long_exposures() -> None:
    fake_sdk_camera = FakeAsiCamera()
    camera = _build_camera(250.0, fake_sdk_camera)

    frame = camera.capture_frame()

    assert frame is not None
    assert fake_sdk_camera.calls.count("capture") == 1
    assert fake_sdk_camera.calls.count("capture_video_frame") == 0


def test_zwo_asi_camera_falls_back_to_single_capture_when_video_fails() -> None:
    fake_sdk_camera = FakeAsiCamera()
    fake_sdk_camera.video_frame = np.array([], dtype=np.uint8)
    camera = _build_camera(10.0, fake_sdk_camera)

    frame = camera.capture_frame()

    assert frame is not None
    assert fake_sdk_camera.calls.count("capture_video_frame") == 1
    assert fake_sdk_camera.calls.count("capture") == 1
    assert fake_sdk_camera.calls.count("stop_video_capture") == 1


def test_zwo_asi_camera_sets_exposure_and_gain_before_capture() -> None:
    fake_sdk_camera = FakeAsiCamera()
    camera = _build_camera(150.0, fake_sdk_camera)

    frame = camera.capture_frame()

    assert frame is not None
    assert f"set_control:{asi.ASI_EXPOSURE}:150000" in fake_sdk_camera.calls
    assert f"set_control:{asi.ASI_GAIN}:300" in fake_sdk_camera.calls
    assert f"set_image_type:{asi.ASI_IMG_Y8}" in fake_sdk_camera.calls
    assert "capture_video_frame_timeout:800" in fake_sdk_camera.calls
