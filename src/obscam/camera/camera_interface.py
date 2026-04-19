#!/usr/bin/env python3
"""Camera interface protocol defining the contract for all camera implementations."""

from typing import Any, Protocol, TypedDict


class FrameMetadata(TypedDict):
    """Metadata for a captured frame."""

    exposure_ms: float
    gain: int
    timestamp: float


class CameraInterface(Protocol):
    """Protocol defining pure camera hardware control interface."""

    def connect(self) -> bool:
        """Connect to the camera hardware.

        Returns:
            True if connection successful, False otherwise.
        """
        ...

    def disconnect(self) -> None:
        """Disconnect from the camera hardware."""
        ...

    def interrupt_capture(self) -> None:
        """Interrupt an in-progress capture if the backend is stopping."""
        ...

    def get_status(self) -> dict[str, Any]:
        """Get current camera status including connection state and settings.

        Returns:
            Dictionary containing camera status information.
        """
        ...

    def capture_frame(self) -> bytes | None:
        """Capture a single frame and return as JPEG bytes.

        Returns:
            JPEG bytes of captured frame, or None if capture failed.
        """
        ...

    def update_settings(self, **settings: Any) -> bool:
        """Update camera settings for the next capture.

        Args:
            **settings: Camera settings such as exposure_ms and gain.

        Returns:
            True if settings updated successfully, False otherwise.
        """
        ...

    def get_current_settings(self) -> dict[str, Any]:
        """Get the current camera settings.

        Returns:
            Dictionary of current camera settings.
        """
        ...

    def get_control_capabilities(self) -> dict[str, Any]:
        """Get camera control capabilities and ranges.

        Returns:
            Dictionary containing control limits for exposure and gain.
        """
        ...
