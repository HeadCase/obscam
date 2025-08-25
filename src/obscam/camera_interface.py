#!/usr/bin/env python3
"""Camera interface protocol defining the contract for all camera
implementations."""

from typing import Any, Protocol, TypedDict


class FrameMetadata(TypedDict):
    """Metadata for a captured frame."""

    exposure_ms: float
    gain: int
    wb_r: int | None
    wb_b: int | None
    timestamp: float


class CameraInterface(Protocol):
    """Protocol defining pure camera hardware control interface."""

    def connect(self) -> bool:
        """Connect to the camera hardware.

        Returns:
            True if connection successful, False otherwise
        """
        ...

    def disconnect(self) -> None:
        """Disconnect from the camera hardware."""
        ...

    def get_status(self) -> dict[str, Any]:
        """Get current camera status including connection state and settings.

        Returns:
            Dictionary containing camera status information
        """
        ...

    def capture_frame(self) -> bytes | None:
        """Capture a single frame and return as JPEG bytes.

        Returns:
            JPEG bytes of captured frame, or None if capture failed
        """
        ...

    def update_settings(self, **settings: Any) -> bool:
        """Update camera settings for the next capture.

        Args:
            **settings: Camera settings (exposure_ms, gain, wb_r, wb_b)

        Returns:
            True if settings updated successfully, False otherwise
        """
        ...

    def get_current_settings(self) -> FrameMetadata:
        """Get the current camera settings.

        Returns:
            Dictionary of current camera settings
        """
        ...
