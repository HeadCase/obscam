"""Corrected buffer-reusing python-zwoasi backend for GRE-183."""

from __future__ import annotations

import ctypes
from collections.abc import Mapping
from typing import Protocol, cast

import zwoasi as asi  # pyright: ignore[reportMissingTypeStubs]

from obscam.tools.gre_183_prototype.contract import (
    CameraCapabilities,
    ControlCapability,
    ImageFormat,
    RunnerKind,
    Scenario,
)

REQUIRED_CAMERA_MODEL = "ASI662MC"
DEFAULT_SDK_LIBRARY = "/usr/local/lib/libASICamera2.so"


class ZwoCamera(Protocol):
    """Typed isolation seam around the untyped python-zwoasi camera."""

    def get_camera_property(self) -> Mapping[str, object]: ...

    def get_controls(self) -> Mapping[str, Mapping[str, object]]: ...

    def set_control_value(
        self, control_type: int, value: int, auto: bool = False
    ) -> None: ...

    def set_roi_format(
        self, width: int, height: int, bins: int, image_type: int
    ) -> None: ...

    def set_roi_start_position(self, start_x: int, start_y: int) -> None: ...

    def start_video_capture(self) -> None: ...

    def stop_video_capture(self) -> None: ...

    def stop_exposure(self) -> None: ...

    def get_video_data(self, timeout: int, buffer_: bytearray) -> bytearray: ...

    def get_dropped_frames(self) -> int: ...

    def close(self) -> None: ...


def _integer(value: object, field_name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError(f"SDK {field_name} must be an integer")
    return value


def _boolean(value: object, field_name: str) -> bool:
    if not isinstance(value, bool):
        raise TypeError(f"SDK {field_name} must be a boolean")
    return value


def _text(value: object, field_name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"SDK {field_name} must be text")
    return value


def _sdk_version(library_path: str) -> str:
    library = ctypes.CDLL(library_path)
    get_version = library.ASIGetSDKVersion
    get_version.argtypes = []
    get_version.restype = ctypes.c_char_p
    raw_version = get_version()
    if raw_version is None:
        raise RuntimeError("ASIGetSDKVersion returned no version")
    return raw_version.decode("ascii")


def _find_camera(required_model: str) -> ZwoCamera:
    for camera_index in range(asi.get_num_cameras()):
        camera = cast(ZwoCamera, asi.Camera(camera_index))
        camera_property = camera.get_camera_property()
        camera_name = _text(camera_property.get("Name"), "camera name")
        if required_model in camera_name:
            return camera
        camera.close()
    raise RuntimeError(f"required camera {required_model} not found")


def _control_by_type(
    controls: Mapping[str, Mapping[str, object]], control_type: int
) -> Mapping[str, object] | None:
    for control in controls.values():
        if _integer(control.get("ControlType"), "control type") == control_type:
            return control
    return None


def _control_capability(control: Mapping[str, object]) -> ControlCapability:
    return ControlCapability(
        minimum=_integer(control.get("MinValue"), "control minimum"),
        maximum=_integer(control.get("MaxValue"), "control maximum"),
        default=_integer(control.get("DefaultValue"), "control default"),
        writable=_boolean(control.get("IsWritable"), "control writable flag"),
    )


def _integer_list(value: object, field_name: str) -> list[int]:
    if not isinstance(value, list):
        raise TypeError(f"SDK {field_name} must be a list")
    return [_integer(item, field_name) for item in value]


def discover_capabilities(
    *,
    library_path: str = DEFAULT_SDK_LIBRARY,
    required_model: str = REQUIRED_CAMERA_MODEL,
) -> CameraCapabilities:
    """Read and normalize the binding camera's relevant SDK capabilities."""
    asi.init(library_path)
    camera = _find_camera(required_model)
    try:
        camera_property = camera.get_camera_property()
        controls = camera.get_controls()
        bandwidth = _control_by_type(controls, asi.ASI_BANDWIDTHOVERLOAD)
        if bandwidth is None:
            raise RuntimeError("camera does not expose ASI_BANDWIDTHOVERLOAD")
        high_speed = _control_by_type(controls, asi.ASI_HIGH_SPEED_MODE)
        raw_formats = _integer_list(
            camera_property.get("SupportedVideoFormat"),
            "supported video formats",
        )
        formats_by_value = {
            image_format.sdk_value: image_format for image_format in ImageFormat
        }
        return CameraCapabilities(
            camera_name=_text(camera_property.get("Name"), "camera name"),
            sdk_version=_sdk_version(library_path),
            width=_integer(camera_property.get("MaxWidth"), "maximum width"),
            height=_integer(camera_property.get("MaxHeight"), "maximum height"),
            is_usb3_host=_boolean(camera_property.get("IsUSB3Host"), "USB 3 host flag"),
            is_usb3_camera=_boolean(
                camera_property.get("IsUSB3Camera"), "USB 3 camera flag"
            ),
            supported_formats=[
                formats_by_value[value]
                for value in raw_formats
                if value in formats_by_value
            ],
            bandwidth=_control_capability(bandwidth),
            high_speed=(
                _control_capability(high_speed) if high_speed is not None else None
            ),
        )
    finally:
        camera.close()


class PythonZwoasiBackend:
    """Direct python-zwoasi video capture with a caller-owned buffer."""

    def __init__(
        self,
        *,
        library_path: str = DEFAULT_SDK_LIBRARY,
        required_model: str = REQUIRED_CAMERA_MODEL,
    ) -> None:
        """Open the binding camera without configuring capture yet."""
        asi.init(library_path)
        self._camera = _find_camera(required_model)
        camera_property = self._camera.get_camera_property()
        self._camera_name = _text(camera_property.get("Name"), "camera name")
        self._sdk_version = _sdk_version(library_path)
        self._started = False

    @property
    def runner_kind(self) -> RunnerKind:
        """Return the implementation identity."""
        return RunnerKind.PYTHON_ZWOASI

    @property
    def sdk_version(self) -> str:
        """Return the loaded SDK version."""
        return self._sdk_version

    @property
    def camera_name(self) -> str:
        """Return the selected camera name."""
        return self._camera_name

    def start(self, scenario: Scenario) -> None:
        """Configure controls and start full-frame video acquisition."""
        try:
            self._camera.stop_video_capture()
        except asi.ZWO_Error:
            pass
        try:
            self._camera.stop_exposure()
        except asi.ZWO_Error:
            pass

        self._camera.set_control_value(asi.ASI_EXPOSURE, scenario.exposure_us)
        self._camera.set_control_value(asi.ASI_GAIN, scenario.gain)
        self._camera.set_control_value(asi.ASI_HIGH_SPEED_MODE, scenario.high_speed)
        self._camera.set_control_value(asi.ASI_BANDWIDTHOVERLOAD, scenario.bandwidth)
        self._camera.set_roi_format(
            scenario.width,
            scenario.height,
            1,
            scenario.image_format.sdk_value,
        )
        self._camera.set_roi_start_position(0, 0)
        self._camera.start_video_capture()
        self._started = True

    def capture_into(self, buffer: bytearray, timeout_ms: int) -> None:
        """Fill the reused caller-owned buffer without constructing NumPy views."""
        self._camera.get_video_data(timeout_ms, buffer)

    def dropped_frames(self) -> int:
        """Return the SDK dropped-frame counter."""
        return self._camera.get_dropped_frames()

    def stop(self) -> None:
        """Stop video capture if it was started."""
        if not self._started:
            return
        try:
            self._camera.stop_video_capture()
        finally:
            self._started = False

    def close(self) -> None:
        """Close the SDK camera handle."""
        try:
            self.stop()
        finally:
            self._camera.close()
