"""Strict media-capability preflight for GRE-190."""

from __future__ import annotations

import json
import shutil
import subprocess
from dataclasses import asdict, dataclass


@dataclass(frozen=True, slots=True)
class CapabilityReport:
    """Installed capabilities that gate valid finalist measurements."""

    gstreamer: bool
    webrtcbin: bool
    jpegenc: bool
    mp4mux: bool
    hardware_h264_elements: tuple[str, ...]
    video_devices: tuple[str, ...]
    h264_webrtc_ready: bool
    blockers: tuple[str, ...]

    def to_json(self) -> str:
        """Serialize the report for evidence capture."""
        return json.dumps(asdict(self), indent=2)


def inspect_capabilities() -> CapabilityReport:
    """Inspect without opening cameras or changing host state."""
    gst_inspect = shutil.which("gst-inspect-1.0")
    elements = {
        name: _gst_element_exists(gst_inspect, name)
        for name in (
            "webrtcbin",
            "jpegenc",
            "mp4mux",
            "v4l2h264enc",
            "omxh264enc",
            "rpiav1enc",
        )
    }
    hardware = tuple(
        name for name in ("v4l2h264enc", "omxh264enc", "rpiav1enc") if elements[name]
    )
    video_devices = tuple(
        str(path) for path in sorted(__import__("pathlib").Path("/dev").glob("video*"))
    )
    blockers: list[str] = []
    if not elements["webrtcbin"]:
        blockers.append("GStreamer webrtcbin is unavailable")
    if not hardware:
        blockers.append("no GStreamer hardware H.264 encoder element was found")
    if not video_devices:
        blockers.append("no V4L2 video/codec devices were found")
    ready = elements["webrtcbin"] and bool(hardware) and bool(video_devices)
    return CapabilityReport(
        gstreamer=gst_inspect is not None,
        webrtcbin=elements["webrtcbin"],
        jpegenc=elements["jpegenc"],
        mp4mux=elements["mp4mux"],
        hardware_h264_elements=hardware,
        video_devices=video_devices,
        h264_webrtc_ready=ready,
        blockers=tuple(blockers),
    )


def _gst_element_exists(gst_inspect: str | None, element: str) -> bool:
    """Return whether GStreamer can load an element."""
    if gst_inspect is None:
        return False
    result = subprocess.run(
        [gst_inspect, element],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return result.returncode == 0
