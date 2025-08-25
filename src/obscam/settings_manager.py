#!/usr/bin/env python3
"""Settings persistence manager with atomic file operations."""

import json
import queue
import threading
from pathlib import Path
from typing import Any

from obscam.logging_config import get_logger

logger = get_logger("settings_manager")


class SettingsManager:
    """Handles loading and saving camera settings to disk with thread-safe queuing."""

    def __init__(self, cache_dir: Path):
        self.settings_file = cache_dir / "obscam_settings.json"
        self.cache_dir = cache_dir

        # Thread-safe queue for settings save requests
        self._save_queue: queue.Queue[dict[str, Any]] = queue.Queue()
        self._save_thread: threading.Thread | None = None
        self._running = False

        # Ensure cache directory exists
        cache_dir.mkdir(parents=True, exist_ok=True)

        logger.info(
            "Settings manager initialized", settings_file=str(self.settings_file)
        )

    def start(self) -> None:
        """Start background thread for async settings saves."""
        if self._running:
            return

        self._running = True
        self._save_thread = threading.Thread(
            target=self._save_worker, daemon=True, name="SettingsSaveWorker"
        )
        self._save_thread.start()
        logger.info("Settings save worker started")

    def stop(self) -> None:
        """Stop background save thread."""
        if self._running:
            self._running = False
            # Signal thread to wake up and exit
            self._save_queue.put({})
            if self._save_thread:
                self._save_thread.join(timeout=1.0)
            logger.info("Settings save worker stopped")

    def load_settings(self) -> dict[str, Any] | None:
        """Load settings from disk."""
        try:
            if self.settings_file.exists():
                with open(self.settings_file, "r") as f:
                    settings = json.load(f)
                logger.info("Settings loaded successfully", settings=settings)
                return settings
            else:
                logger.info("No settings file found, using defaults")
        except Exception as e:
            logger.error("Failed to load settings", error=str(e))
        return None

    def save_settings_async(self, settings: dict[str, Any]) -> None:
        """Queue settings for async saving to disk."""
        try:
            # Non-blocking put to queue latest settings
            try:
                # Clear queue and add latest settings
                while not self._save_queue.empty():
                    self._save_queue.get_nowait()
            except queue.Empty:
                pass

            self._save_queue.put_nowait(settings)
            logger.debug("Settings queued for save", settings=settings)
        except queue.Full:
            logger.warning("Settings save queue full, dropping save request")

    def save_settings_sync(self, settings: dict[str, Any]) -> None:
        """Synchronously save settings to disk (for shutdown/critical saves)."""
        self._save_settings_to_disk(settings)

    def _save_worker(self) -> None:
        """Background worker thread for settings saves."""
        logger.debug("Settings save worker thread started")

        while self._running:
            try:
                # Block waiting for settings to save
                settings = self._save_queue.get(timeout=1.0)

                # Check if this is a shutdown signal
                if not self._running or not settings:
                    break

                self._save_settings_to_disk(settings)

            except queue.Empty:
                # Timeout - continue loop to check _running
                continue
            except Exception as e:
                logger.error("Settings save worker error", error=str(e))

        logger.debug("Settings save worker thread ended")

    def _save_settings_to_disk(self, settings: dict[str, Any]) -> None:
        """Atomically save settings to disk."""
        try:
            # Use atomic write: write to temp file, then rename
            temp_file = self.settings_file.with_suffix(".tmp")

            with open(temp_file, "w") as f:
                json.dump(settings, f, indent=2)

            # Atomic rename on Unix systems
            temp_file.rename(self.settings_file)

            logger.info("Settings saved successfully", settings=settings)

        except Exception as e:
            logger.error("Failed to save settings", settings=settings, error=str(e))
            # Clean up temp file if it exists
            temp_file = self.settings_file.with_suffix(".tmp")
            if temp_file.exists():
                try:
                    temp_file.unlink()
                except:
                    pass
