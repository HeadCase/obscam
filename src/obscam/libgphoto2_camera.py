#!/usr/bin/env python3
"""basic_capture.py — Minimal Nikon Zf capture via python-gphoto2.

python-gphoto2 reference: https://github.com/jim-easterbrook/python-gphoto2
libgphoto2 reference: https://github.com/gphoto/libgphoto2
"""

import io
import time

import math
from pathlib import Path

from PIL import Image
import gphoto2 as gp


class Gphoto2Camera:
    def __init__(self) -> None:
        self.camera = gp.Camera()
        self.inited = False

    def init(self) -> None:
        if not self.inited:
            self.camera.init()
            self.inited = True

    def exit(self) -> None:
        if self.inited:
            self.camera.exit()
            self.inited = False

    def _fresh_cfg(self) -> gp.CameraWidget:
        return self.camera.get_config()

    def _child(self, key: str) -> gp.CameraWidget:
        return self._fresh_cfg().get_child_by_name(key)

    def _get_image_buffer(
        self, folder: str, name: str, file_type: int = gp.GP_FILE_TYPE_NORMAL
    ) -> bytes:
        """Download a camera file into memory and return raw bytes."""
        cam_file = self.camera.file_get(folder, name, file_type)

        return io.BytesIO(cam_file.get_data_and_size())

    def _has_writable_bulb(self) -> bool:
        """Return True if a writable 'bulb' control exists."""
        try:
            cfg = self._fresh_cfg()
            bulb = cfg.get_child_by_name("bulb")

            cur = bulb.get_value()
            bulb.set_value(cur)
            self.camera.set_config(cfg)
            return True
        except gp.GPhoto2Error:
            return False

    def _shutter_mode_hint(self) -> str | None:
        """Return 'Bulb', 'Time', or None based on the current shutterspeed
        readout."""
        try:
            val = str(self.current_shutterspeed()).strip().lower()
        except gp.GPhoto2Error:
            return None
        if "bulb" in val:
            return "Bulb"
        if "time" in val:  # some Nikons display 'Time' or 'T'
            return "Time"
        return None

    def _wait_for_file_added(self, timeout_s: float = 30.0) -> tuple[str, str]:
        """Wait for the camera to report a newly created file after an
        exposure.

        Returns (folder, name). Raises on timeout.
        """
        deadline = time.time() + timeout_s
        while time.time() < deadline:
            ev_type, ev_data = self.camera.wait_for_event(1000)  # ms
            if ev_type == gp.GP_EVENT_FILE_ADDED:
                return ev_data.folder, ev_data.name

        raise TimeoutError("Timed out waiting for FILE_ADDED event from camera")

    def _set_bulb(self, value: int) -> None:
        """Low-level toggle for the 'bulb' control: 1=open, 0=close."""
        cfg = self._fresh_cfg()
        node = cfg.get_child_by_name("bulb")
        node.set_value(int(value))
        self.camera.set_config(cfg)

    def current_shutterspeed(self) -> str:
        return self._child("shutterspeed").get_value()

    def capture_bulb_image(
        self,
        seconds: float,
        post_timeout_s: float = 30.0,
    ) -> bytes:
        """Perform a Bulb/Time exposure for `seconds` and download the result
        to `out`.

        Args:
          seconds: exposure duration (must be >0). For 'Time', this is the between-toggles delay.
          mode: force 'Bulb' or 'Time'; if None, inferred from current shutter readout.
          post_timeout_s: how long to wait for the file event after closing.

        Returns:
          Absolute Path to the saved file.
        """
        if seconds <= 0:
            raise ValueError("seconds must be > 0")

        if not self._has_writable_bulb():
            raise RuntimeError(
                "Camera does not expose a writable 'bulb' control over PTP"
            )

        hint = self._shutter_mode_hint()
        if hint != "Bulb":
            raise RuntimeError(
                f"Shutter not set to Bulb (current: {hint or 'unknown'}). "
                "Set the dial to B and try again."
            )

        self._set_bulb(1)
        try:
            time.sleep(seconds)
        finally:
            try:
                self._set_bulb(0)
            except Exception as e:
                raise e

        folder, name = self._wait_for_file_added(timeout_s=post_timeout_s)
        cam_file = self.camera.file_get(folder, name, gp.GP_FILE_TYPE_NORMAL)

        return self._get_image_buffer(folder, name)

    def save_jpeg(self, data: bytes, out: Path) -> Path:
        """Trigger capture and save a JPEG to 'out' (overwrites if exists)."""
        img = Image.open(data)
        img.save(out)

        return out.resolve()


if __name__ == "__main__":
    main()
