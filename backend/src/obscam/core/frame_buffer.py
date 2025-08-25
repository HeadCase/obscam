#!/usr/bin/env python3
"""Thread-safe frame buffer using queue for latest image storage."""

import queue
from typing import Any

from obscam.common.logging_config import get_logger

logger = get_logger("frame_buffer")


class LatestFrameBuffer:
    """Thread-safe storage for the most recent captured frame using queues."""

    def __init__(self):
        # Use maxsize=1 queue to automatically drop old frames
        self._frame_queue: queue.Queue[tuple[bytes, dict[str, Any]]] = queue.Queue(
            maxsize=1
        )
        # Callback for new frame notifications (for MJPEG/SSE)
        self.on_new_frame = None
        logger.debug("Frame buffer initialized with queue-based storage")

    def update_frame(self, frame_bytes: bytes, metadata: dict[str, Any]) -> None:
        """Update the latest frame and metadata."""
        try:
            # Non-blocking put - if queue is full, drop old frame
            self._frame_queue.put_nowait((frame_bytes, metadata))
            logger.debug(
                "Frame updated",
                frame_size=len(frame_bytes),
                timestamp=metadata.get("timestamp"),
                exposure_ms=metadata.get("exposure_ms"),
            )
        except queue.Full:
            # Queue is full (has 1 item), so remove old and add new
            try:
                self._frame_queue.get_nowait()  # Drop old frame
                self._frame_queue.put_nowait((frame_bytes, metadata))
                logger.debug(
                    "Frame updated (replaced old)",
                    frame_size=len(frame_bytes),
                    timestamp=metadata.get("timestamp"),
                )
            except queue.Empty:
                # Race condition - queue became empty, try again
                try:
                    self._frame_queue.put_nowait((frame_bytes, metadata))
                except queue.Full:
                    logger.warning("Failed to update frame due to queue contention")

        # Notify listeners about new frame (for MJPEG/SSE)
        if self.on_new_frame:
            try:
                self.on_new_frame()
            except Exception as e:
                logger.error("Error in new frame callback", error=str(e))

    def get_latest_frame(self) -> bytes | None:
        """Get the latest frame bytes without removing from queue."""
        try:
            frame_bytes, metadata = self._frame_queue.get_nowait()
            # Put it back for other consumers
            self._frame_queue.put_nowait((frame_bytes, metadata))
            logger.debug("Frame retrieved", frame_size=len(frame_bytes))
            return frame_bytes
        except queue.Empty:
            logger.debug("No frame available")
            return None
        except queue.Full:
            # This shouldn't happen with our design but handle gracefully
            logger.warning("Queue contention during frame retrieval")
            return None

    def get_frame_metadata(self) -> dict[str, Any] | None:
        """Get metadata for the latest frame without removing from queue."""
        try:
            frame_bytes, metadata = self._frame_queue.get_nowait()
            # Put it back for other consumers
            self._frame_queue.put_nowait((frame_bytes, metadata))
            return metadata
        except queue.Empty:
            return None
        except queue.Full:
            logger.warning("Queue contention during metadata retrieval")
            return None

    def get_frame_with_metadata(self) -> tuple[bytes, dict[str, Any]] | None:
        """Get both frame and metadata without removing from queue."""
        try:
            frame_data = self._frame_queue.get_nowait()
            # Put it back for other consumers
            self._frame_queue.put_nowait(frame_data)
            return frame_data
        except queue.Empty:
            return None
        except queue.Full:
            logger.warning("Queue contention during combined retrieval")
            return None

    def has_frame(self) -> bool:
        """Check if a frame is available."""
        return not self._frame_queue.empty()

    def clear(self) -> None:
        """Clear the frame buffer."""
        try:
            while True:
                self._frame_queue.get_nowait()
        except queue.Empty:
            pass
        logger.debug("Frame buffer cleared")
