#!/usr/bin/env python3
"""Synthetic camera implementation backed by fixture images."""

import io
import math
import os
import threading
import time
from pathlib import Path
from typing import Any

import numpy as np
from PIL import Image, ImageDraw, ImageFont, ImageOps

from obscam.common.constants import ASSETS_DIR
from obscam.common.logging_config import get_logger

from .camera_interface import CameraInterface

logger = get_logger("synthetic_camera")

DEFAULT_SYNTHETIC_FRAME_DIR = ASSETS_DIR / "test_loop"
SUPPORTED_EXTENSIONS = (".jpg", ".jpeg", ".png")
DEFAULT_EXPOSURE_MS = 200.0
DEFAULT_GAIN = 250
MAX_JITTER_PX = 12
MIN_CAPTURE_INTERVAL_S = 0.2


class SyntheticCamera(CameraInterface):
    """Synthetic camera using captured observatory frames as a looping feed."""

    def __init__(self, frame_dir: str | Path | None = None):
        self.frame_dir = Path(
            frame_dir
            or os.getenv("OBSCAM_SYNTHETIC_FRAME_DIR", DEFAULT_SYNTHETIC_FRAME_DIR)
        )
        self.settings_lock = threading.Lock()
        self.current_settings: dict[str, float | int] = {
            "exposure_ms": DEFAULT_EXPOSURE_MS,
            "gain": DEFAULT_GAIN,
        }
        self.connected = False
        self.frame_index = 0
        self.fixture_frames: list[Image.Image] = []
        self.frame_paths: list[Path] = []
        self.rng = np.random.default_rng(662)
        self._font = ImageFont.load_default()

    def connect(self) -> bool:
        """Load fixture frames and make the synthetic camera available."""
        try:
            self.frame_paths = self._discover_frame_paths()
            self.fixture_frames = [self._load_frame(path) for path in self.frame_paths]
            self.frame_index = 0
            self.connected = True
            logger.info(
                "Synthetic camera connected",
                frame_dir=str(self.frame_dir),
                frame_count=len(self.fixture_frames),
            )
            return True
        except Exception as exc:
            logger.error("Failed to initialize synthetic camera", error=str(exc))
            self.connected = False
            self.fixture_frames = []
            self.frame_paths = []
            return False

    def disconnect(self) -> None:
        """Disconnect the synthetic camera."""
        self.connected = False
        self.fixture_frames = []
        self.frame_paths = []
        logger.info("Synthetic camera disconnected")

    def get_status(self) -> dict[str, Any]:
        """Get synthetic camera status and current settings."""
        with self.settings_lock:
            settings = self.current_settings.copy()

        return {
            "status": "connected" if self.connected else "disconnected",
            "camera_model": "Synthetic Camera",
            "current_exposure_ms": settings["exposure_ms"],
            "current_gain": settings["gain"],
            "frame_source": str(self.frame_dir),
            "fixture_frame_count": len(self.fixture_frames),
        }

    def capture_frame(self) -> bytes | None:
        """Render the next synthetic frame and return it as JPEG bytes."""
        if not self.connected or not self.fixture_frames:
            return None

        with self.settings_lock:
            exposure_ms = float(self.current_settings["exposure_ms"])
            gain = int(self.current_settings["gain"])

        time.sleep(max(MIN_CAPTURE_INTERVAL_S, exposure_ms / 1000.0))

        base_frame = self.fixture_frames[
            self.frame_index % len(self.fixture_frames)
        ].copy()
        rendered = self._apply_camera_effects(base_frame, exposure_ms, gain)
        rendered = self._annotate_frame(rendered, exposure_ms, gain)

        buffer = io.BytesIO()
        rendered.save(buffer, format="JPEG", quality=85)
        self.frame_index += 1
        return buffer.getvalue()

    def update_settings(self, **settings: Any) -> bool:
        """Update synthetic camera settings."""
        try:
            with self.settings_lock:
                if "exposure_ms" in settings:
                    self.current_settings["exposure_ms"] = float(
                        settings["exposure_ms"]
                    )
                if "gain" in settings:
                    self.current_settings["gain"] = int(settings["gain"])
            return True
        except Exception as exc:
            logger.error("Failed to update synthetic camera settings", error=str(exc))
            return False

    def get_current_settings(self) -> dict[str, Any]:
        """Get the current synthetic camera settings."""
        with self.settings_lock:
            return self.current_settings.copy()

    def get_control_capabilities(self) -> dict[str, Any]:
        """Expose a ZWO-like control surface for local frontend development."""
        return {
            "exposure_ms": {"min": 0.032, "max": 30000.0, "type": "float"},
            "gain": {"min": 0, "max": 600, "type": "int"},
            "camera_type": "Synthetic Camera",
        }

    def _discover_frame_paths(self) -> list[Path]:
        if not self.frame_dir.exists():
            self.frame_dir.mkdir(parents=True)
            # raise FileNotFoundError(
            #     f"Synthetic frame directory not found: {self.frame_dir}"
            # )

        frame_paths = sorted(
            path
            for path in self.frame_dir.iterdir()
            if path.is_file() and path.suffix.lower() in SUPPORTED_EXTENSIONS
        )
        if not frame_paths:
            raise FileNotFoundError(
                f"No synthetic fixture images found in {self.frame_dir}"
            )
        return frame_paths

    def _load_frame(self, path: Path) -> Image.Image:
        with Image.open(path) as image:
            return ImageOps.grayscale(image).copy()

    def _apply_camera_effects(
        self, frame: Image.Image, exposure_ms: float, gain: int
    ) -> Image.Image:
        width, height = frame.size
        crop_margin = MAX_JITTER_PX
        phase = self.frame_index / max(1, len(self.fixture_frames))
        shift_x = round(math.sin(phase * math.tau * 1.3) * crop_margin)
        shift_y = round(math.cos(phase * math.tau * 1.7) * crop_margin)
        crop_box = (
            crop_margin + shift_x,
            crop_margin + shift_y,
            width - crop_margin + shift_x,
            height - crop_margin + shift_y,
        )
        jittered = frame.crop(crop_box).resize(frame.size, Image.Resampling.BICUBIC)

        frame_array = np.asarray(jittered, dtype=np.float32)
        exposure_factor = min(2.2, max(0.45, 0.65 + math.log10(exposure_ms + 10.0)))
        gain_factor = 1.0 + (gain / 600.0) * 0.55
        brightness = frame_array * exposure_factor * gain_factor

        noise_scale = 2.0 + (gain / 600.0) * 16.0
        noise = self.rng.normal(loc=0.0, scale=noise_scale, size=frame_array.shape)
        brightness += noise

        clipped = np.clip(brightness, 0, 255).astype(np.uint8)
        return Image.fromarray(clipped)

    def _annotate_frame(
        self, frame: Image.Image, exposure_ms: float, gain: int
    ) -> Image.Image:
        overlay = frame.convert("RGB")
        draw = ImageDraw.Draw(overlay)
        timestamp = time.strftime("%H:%M:%S")
        status_lines = [
            "SYNTHETIC",
            f"frame {self.frame_index + 1:04d}/{len(self.fixture_frames):04d}",
            f"{timestamp}  exp {exposure_ms:.1f}ms  gain {gain}",
        ]
        text = "\n".join(status_lines)
        padding = 10
        text_box = draw.multiline_textbbox((0, 0), text, font=self._font, spacing=4)
        box_width = text_box[2] - text_box[0] + padding * 2
        box_height = text_box[3] - text_box[1] + padding * 2
        draw.rounded_rectangle(
            (12, 12, 12 + box_width, 12 + box_height),
            radius=8,
            fill=(0, 0, 0),
        )
        draw.multiline_text(
            (12 + padding, 12 + padding),
            text,
            font=self._font,
            fill=(235, 240, 255),
            spacing=4,
        )

        return ImageOps.grayscale(overlay)
