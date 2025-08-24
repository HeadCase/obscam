#!/usr/bin/env python3
"""Camera interface protocol defining the contract for all camera implementations."""

from typing import Any, Protocol, TypedDict


class FrameMetadata(TypedDict):
    """Metadata for a captured frame."""

    exposure_ms: float
    gain: int
    wb_r: int | None
    wb_b: int | None
    timestamp: float


class CameraInterface(Protocol):
    """Protocol defining the interface all camera implementations must follow."""

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

    def start_continuous_capture(self) -> bool:
        """Start the continuous capture loop.

        Returns:
            True if capture started successfully, False otherwise
        """
        ...

    def stop_continuous_capture(self) -> None:
        """Stop the continuous capture loop."""
        ...

    def get_latest_frame(self) -> bytes | None:
        """Get the most recently captured frame as JPEG bytes.

        Returns:
            JPEG bytes of the latest frame, or None if no frame available
        """
        ...

    def get_frame_metadata(self) -> FrameMetadata | None:
        """Get metadata for the most recently captured frame.

        Returns:
            Metadata dictionary for the latest frame, or None if no frame available
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

    def get_current_settings(self) -> dict[str, Any]:
        """Get the current camera settings.

        Returns:
            Dictionary of current camera settings
        """
        ...
