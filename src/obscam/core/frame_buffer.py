#!/usr/bin/env python3
"""Thread-safe latest-frame storage for camera output."""

import threading
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from types import MappingProxyType
from typing import Any

from obscam.common.logging_config import get_logger

logger = get_logger("frame_buffer")


@dataclass(frozen=True, slots=True)
class FrameSnapshot:
    """Immutable latest-frame snapshot for consumers."""

    frame_bytes: bytes
    metadata: Mapping[str, Any]
    generation: int


class LatestFrameBuffer:
    """Thread-safe storage for the most recent captured frame."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._latest_snapshot: FrameSnapshot | None = None
        self._next_generation = 0
        self.on_new_frame: Callable[[FrameSnapshot], None] | None = None
        logger.debug("Frame buffer initialized with snapshot-based storage")

    def update_frame(self, frame_bytes: bytes, metadata: dict[str, Any]) -> None:
        """Update the latest frame and metadata."""
        frozen_metadata = MappingProxyType(dict(metadata))
        with self._lock:
            self._next_generation += 1
            snapshot = FrameSnapshot(
                frame_bytes=frame_bytes,
                metadata=frozen_metadata,
                generation=self._next_generation,
            )
            self._latest_snapshot = snapshot

        logger.debug(
            "Frame updated",
            frame_size=len(frame_bytes),
            timestamp=metadata.get("timestamp"),
            exposure_ms=metadata.get("exposure_ms"),
            generation=snapshot.generation,
        )

        if self.on_new_frame:
            try:
                self.on_new_frame(snapshot)
            except Exception as e:
                logger.error("Error in new frame callback", error=str(e))

    def get_latest_frame(self) -> bytes | None:
        """Get the latest frame bytes without consuming the snapshot."""
        snapshot = self.get_latest_snapshot()
        if snapshot is None:
            logger.debug("No frame available")
            return None

        logger.debug(
            "Frame retrieved",
            frame_size=len(snapshot.frame_bytes),
            generation=snapshot.generation,
        )
        return snapshot.frame_bytes

    def get_frame_metadata(self) -> dict[str, Any] | None:
        """Get metadata for the latest frame without consuming the snapshot."""
        snapshot = self.get_latest_snapshot()
        if snapshot is None:
            return None
        return dict(snapshot.metadata)

    def get_frame_with_metadata(self) -> tuple[bytes, dict[str, Any]] | None:
        """Get both frame bytes and metadata without consuming the snapshot."""
        snapshot = self.get_latest_snapshot()
        if snapshot is None:
            return None
        return snapshot.frame_bytes, dict(snapshot.metadata)

    def get_latest_snapshot(self) -> FrameSnapshot | None:
        """Get the latest immutable frame snapshot."""
        with self._lock:
            return self._latest_snapshot

    def has_frame(self) -> bool:
        """Check if a frame is available."""
        return self.get_latest_snapshot() is not None

    def clear(self) -> None:
        """Clear the frame buffer."""
        with self._lock:
            self._latest_snapshot = None
        logger.debug("Frame buffer cleared")
