#!/usr/bin/env python3
"""Disk-based frame caching for persistence across restarts."""

import queue
import threading
from pathlib import Path

from obscam.common.logging_config import get_logger

logger = get_logger("frame_cache")


class FrameCache:
    """Handles disk caching of latest frame with async writes for performance."""

    def __init__(self, cache_dir: Path):
        self.frame_cache_file = cache_dir / "obscam_latest_frame.jpg"
        self.cache_dir = cache_dir

        # Thread-safe queue for async disk writes
        self._write_queue: queue.Queue[bytes] = queue.Queue(maxsize=2)
        self._write_thread: threading.Thread | None = None
        self._running = False

        # Ensure cache directory exists
        cache_dir.mkdir(parents=True, exist_ok=True)

        logger.info("Frame cache initialized", cache_file=str(self.frame_cache_file))

    def start(self) -> None:
        """Start background thread for async frame writes."""
        if self._running:
            return

        self._running = True
        self._write_thread = threading.Thread(
            target=self._write_worker, daemon=True, name="FrameCacheWriter"
        )
        self._write_thread.start()
        logger.info("Frame cache writer started")

    def stop(self) -> None:
        """Stop background write thread."""
        if self._running:
            self._running = False
            # Signal thread to wake up and exit
            try:
                self._write_queue.put_nowait(b"")  # Empty signal
            except queue.Full:
                pass

            if self._write_thread:
                self._write_thread.join(timeout=1.0)
            logger.info("Frame cache writer stopped")

    def cache_frame_async(self, frame_bytes: bytes) -> None:
        """Queue frame for async disk caching."""
        if not frame_bytes:
            return

        try:
            # Drop old frames and keep only the latest
            while not self._write_queue.empty():
                try:
                    self._write_queue.get_nowait()
                except queue.Empty:
                    break

            self._write_queue.put_nowait(frame_bytes)
            logger.debug("Frame queued for caching", frame_size=len(frame_bytes))

        except queue.Full:
            # Queue is full, drop the frame - we only care about latest
            logger.debug("Cache write queue full, dropping frame")

    def cache_frame_sync(self, frame_bytes: bytes) -> None:
        """Synchronously cache frame to disk."""
        if not frame_bytes:
            return
        self._write_frame_to_disk(frame_bytes)

    def load_cached_frame(self) -> bytes | None:
        """Load cached frame from disk."""
        try:
            if self.frame_cache_file.exists():
                with open(self.frame_cache_file, "rb") as f:
                    frame_data = f.read()
                logger.debug(
                    "Cached frame loaded from disk", frame_size=len(frame_data)
                )
                return frame_data
            else:
                logger.debug("No cached frame file found")
        except Exception as e:
            logger.error("Failed to load cached frame", error=str(e))
        return None

    def has_cached_frame(self) -> bool:
        """Check if a cached frame exists on disk."""
        return self.frame_cache_file.exists()

    def clear_cache(self) -> None:
        """Remove cached frame file."""
        try:
            if self.frame_cache_file.exists():
                self.frame_cache_file.unlink()
                logger.info("Frame cache cleared")
        except Exception as e:
            logger.error("Failed to clear frame cache", error=str(e))

    def _write_worker(self) -> None:
        """Background worker thread for frame writes."""
        logger.debug("Frame cache writer thread started")

        while self._running:
            try:
                # Block waiting for frames to write
                frame_bytes = self._write_queue.get(timeout=1.0)

                # Check for shutdown signal
                if not self._running or len(frame_bytes) == 0:
                    break

                self._write_frame_to_disk(frame_bytes)

            except queue.Empty:
                # Timeout - continue loop to check _running
                continue
            except Exception as e:
                logger.error("Frame cache writer error", error=str(e))

        logger.debug("Frame cache writer thread ended")

    def _write_frame_to_disk(self, frame_bytes: bytes) -> None:
        """Atomically write frame to disk."""
        try:
            # Use atomic write: write to temp file, then rename
            temp_file = self.frame_cache_file.with_suffix(".tmp")

            with open(temp_file, "wb") as f:
                f.write(frame_bytes)

            # Atomic rename on Unix systems
            temp_file.rename(self.frame_cache_file)

            logger.debug("Frame cached to disk", frame_size=len(frame_bytes))

        except Exception as e:
            logger.error(
                "Failed to cache frame", frame_size=len(frame_bytes), error=str(e)
            )
            # Clean up temp file if it exists
            temp_file = self.frame_cache_file.with_suffix(".tmp")
            if temp_file.exists():
                try:
                    temp_file.unlink()
                except:
                    pass
