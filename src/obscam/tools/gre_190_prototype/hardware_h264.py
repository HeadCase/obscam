"""Strict FFmpeg/V4L2 hardware H.264 gate for GRE-190."""

from __future__ import annotations

import json
import shutil
import subprocess
from dataclasses import asdict, dataclass
from pathlib import Path


@dataclass(frozen=True, slots=True)
class HardwareH264Scenario:
    """One deterministic full-resolution encoder scenario."""

    width: int = 1920
    height: int = 1080
    fps: int = 10
    duration_s: int = 30
    bitrate: str = "8M"
    keyframe_interval: int = 10

    @property
    def frames(self) -> int:
        """Return the exact number of input frames."""
        return self.fps * self.duration_s

    def ffmpeg_arguments(self, output: Path) -> list[str]:
        """Build the hardware-only FFmpeg invocation."""
        return [
            "ffmpeg",
            "-hide_banner",
            "-loglevel",
            "error",
            "-re",
            "-f",
            "lavfi",
            "-i",
            f"testsrc2=size={self.width}x{self.height}:rate={self.fps}",
            "-frames:v",
            str(self.frames),
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "h264_v4l2m2m",
            "-profile:v",
            "578",
            "-b:v",
            self.bitrate,
            "-g",
            str(self.keyframe_interval),
            "-f",
            "h264",
            "-y",
            str(output),
        ]


@dataclass(frozen=True, slots=True)
class HardwareH264Result:
    """Validated output from the deterministic hardware gate."""

    codec_name: str
    width: int
    height: int
    frame_rate: str
    frames: int
    encoded_bytes: int

    def to_json(self) -> str:
        """Serialize the evidence summary."""
        return json.dumps(asdict(self), indent=2)


def run_hardware_gate(
    output: Path,
    scenario: HardwareH264Scenario | None = None,
) -> HardwareH264Result:
    """Encode and validate one deterministic stream without software fallback."""
    scenario = scenario or HardwareH264Scenario()
    if shutil.which("ffmpeg") is None or shutil.which("ffprobe") is None:
        raise RuntimeError("ffmpeg and ffprobe are required")
    subprocess.run(scenario.ffmpeg_arguments(output), check=True)
    probe = subprocess.run(
        [
            "ffprobe",
            "-v",
            "error",
            "-count_frames",
            "-show_entries",
            "stream=codec_name,width,height,r_frame_rate,nb_read_frames",
            "-of",
            "json",
            str(output),
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    stream = json.loads(probe.stdout)["streams"][0]
    result = HardwareH264Result(
        codec_name=stream["codec_name"],
        width=stream["width"],
        height=stream["height"],
        frame_rate=stream["r_frame_rate"],
        frames=int(stream["nb_read_frames"]),
        encoded_bytes=output.stat().st_size,
    )
    if result.codec_name != "h264":
        raise RuntimeError(f"unexpected codec: {result.codec_name}")
    if (result.width, result.height) != (scenario.width, scenario.height):
        raise RuntimeError("encoded resolution does not match the scenario")
    if result.frames != scenario.frames:
        raise RuntimeError("encoded frame count does not match the scenario")
    return result
