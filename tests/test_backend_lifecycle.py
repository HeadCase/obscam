import threading
import time
from pathlib import Path

from obscam.core.backend_service import (
    BackendLifecycleState,
    BackendStatusSnapshot,
    CameraBackendService,
)


class RecoveryTestCamera:
    def __init__(
        self,
        *,
        initial_capture_failures: int,
        reconnect_success: bool = True,
    ) -> None:
        self.connected = False
        self.settings: dict[str, object] = {"exposure_ms": 10.0, "gain": 250}
        self.initial_capture_failures = initial_capture_failures
        self.reconnect_success = reconnect_success
        self.capture_calls = 0
        self.connect_calls = 0
        self.disconnect_calls = 0
        self.interrupt_calls = 0
        self._connect_attempt = 0

    def connect(self) -> bool:
        self._connect_attempt += 1
        self.connect_calls += 1
        if self._connect_attempt > 1 and not self.reconnect_success:
            self.connected = False
            return False
        self.connected = True
        return True

    def disconnect(self) -> None:
        self.disconnect_calls += 1
        self.connected = False

    def interrupt_capture(self) -> None:
        self.interrupt_calls += 1

    def get_status(self) -> dict[str, object]:
        return {
            "status": "connected" if self.connected else "disconnected",
            "camera_model": "RecoveryTestCamera",
        }

    def capture_frame(self) -> bytes | None:
        self.capture_calls += 1
        if self.capture_calls <= self.initial_capture_failures:
            raise RuntimeError("simulated capture failure")
        return b"frame"

    def update_settings(self, **settings: object) -> bool:
        for key, value in settings.items():
            self.settings[key] = value
        return True

    def get_current_settings(self) -> dict[str, object]:
        return self.settings.copy()

    def get_control_capabilities(self) -> dict[str, object]:
        return {
            "exposure_ms": {"min": 0.032, "max": 30000.0, "type": "float"},
            "gain": {"min": 0, "max": 600, "type": "int"},
        }


class LongExposureCamera:
    def __init__(self) -> None:
        self.connected = False
        self.settings: dict[str, object] = {"exposure_ms": 5000.0, "gain": 250}
        self.capture_started = threading.Event()
        self.capture_release = threading.Event()
        self.interrupt_calls = 0
        self.disconnect_calls = 0
        self.interrupt_at: float | None = None
        self.disconnect_at: float | None = None

    def connect(self) -> bool:
        self.connected = True
        return True

    def disconnect(self) -> None:
        self.disconnect_calls += 1
        self.disconnect_at = time.monotonic()
        self.connected = False

    def interrupt_capture(self) -> None:
        self.interrupt_calls += 1
        self.interrupt_at = time.monotonic()
        self.capture_release.set()

    def get_status(self) -> dict[str, object]:
        return {
            "status": "connected" if self.connected else "disconnected",
            "camera_model": "LongExposureCamera",
        }

    def capture_frame(self) -> bytes | None:
        self.capture_started.set()
        if self.capture_release.wait(timeout=10.0):
            return None
        return b"frame"

    def update_settings(self, **settings: object) -> bool:
        for key, value in settings.items():
            self.settings[key] = value
        return True

    def get_current_settings(self) -> dict[str, object]:
        return self.settings.copy()

    def get_control_capabilities(self) -> dict[str, object]:
        return {
            "exposure_ms": {"min": 0.032, "max": 30000.0, "type": "float"},
            "gain": {"min": 0, "max": 600, "type": "int"},
        }


class SlowConnectCamera:
    def __init__(self) -> None:
        self.connected = False
        self.connect_calls = 0
        self.settings: dict[str, object] = {"exposure_ms": 10.0, "gain": 250}

    def connect(self) -> bool:
        self.connect_calls += 1
        time.sleep(0.05)
        self.connected = True
        return True

    def disconnect(self) -> None:
        self.connected = False

    def interrupt_capture(self) -> None:
        return None

    def get_status(self) -> dict[str, object]:
        return {
            "status": "connected" if self.connected else "disconnected",
            "camera_model": "SlowConnectCamera",
        }

    def capture_frame(self) -> bytes | None:
        return b"frame"

    def update_settings(self, **settings: object) -> bool:
        for key, value in settings.items():
            self.settings[key] = value
        return True

    def get_current_settings(self) -> dict[str, object]:
        return self.settings.copy()

    def get_control_capabilities(self) -> dict[str, object]:
        return {
            "exposure_ms": {"min": 0.032, "max": 30000.0, "type": "float"},
            "gain": {"min": 0, "max": 600, "type": "int"},
        }


def _wait_for_state(
    backend: CameraBackendService,
    target_state: str,
    *,
    timeout_s: float,
) -> None:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        if backend.get_backend_state() == target_state:
            return
        time.sleep(0.01)
    raise AssertionError(
        "Timed out waiting for backend state "
        f"{target_state}, got {backend.get_backend_state()}"
    )


def _status(backend: CameraBackendService) -> BackendStatusSnapshot:
    return backend.get_status_snapshot()


def test_capture_failures_trigger_recovery_and_return_to_running(
    tmp_path: Path,
    monkeypatch,
) -> None:
    monkeypatch.setattr(
        "obscam.core.backend_service.INITIAL_RECOVERY_DELAYS_S",
        (0.01, 0.02, 0.05),
    )
    monkeypatch.setattr("obscam.core.backend_service.DEGRADED_RETRY_INTERVAL_S", 0.05)
    camera = RecoveryTestCamera(initial_capture_failures=5, reconnect_success=True)
    backend = CameraBackendService(camera, tmp_path)

    assert backend.start_backend() is True
    _wait_for_state(backend, BackendLifecycleState.RUNNING.value, timeout_s=1.0)

    deadline = time.monotonic() + 1.5
    while time.monotonic() < deadline:
        status = _status(backend)
        if status["capture"]["has_frame"] is True and camera.connect_calls >= 2:
            break
        time.sleep(0.01)
    else:
        raise AssertionError("Expected recovery to reconnect and produce a frame")

    status = _status(backend)
    assert status["backend"]["state"] == "running"
    assert status["backend"]["consecutive_capture_failures"] == 0
    assert camera.disconnect_calls >= 1

    backend.stop_backend()


def test_persistent_recovery_failure_transitions_to_degraded(
    tmp_path: Path,
    monkeypatch,
) -> None:
    monkeypatch.setattr(
        "obscam.core.backend_service.INITIAL_RECOVERY_DELAYS_S",
        (0.01, 0.02, 0.05),
    )
    monkeypatch.setattr("obscam.core.backend_service.DEGRADED_RETRY_INTERVAL_S", 1.0)
    camera = RecoveryTestCamera(initial_capture_failures=5, reconnect_success=False)
    backend = CameraBackendService(camera, tmp_path)

    assert backend.start_backend() is True
    _wait_for_state(backend, BackendLifecycleState.DEGRADED.value, timeout_s=1.0)

    status = _status(backend)
    assert status["backend"]["state"] == "degraded"
    assert status["backend"]["last_error"] == "Recovery reconnect failed"
    assert status["backend"]["last_recovery_attempt_at"] is not None

    backend.stop_backend()


def test_stop_during_long_exposure_interrupts_capture_before_disconnect(
    tmp_path: Path,
) -> None:
    camera = LongExposureCamera()
    backend = CameraBackendService(camera, tmp_path)

    assert backend.start_backend() is True
    assert camera.capture_started.wait(timeout=1.0)

    started_at = time.monotonic()
    backend.stop_backend()
    duration_s = time.monotonic() - started_at

    assert duration_s < 2.0
    assert camera.interrupt_calls == 1
    assert camera.disconnect_calls == 1
    assert camera.interrupt_at is not None
    assert camera.disconnect_at is not None
    assert camera.disconnect_at >= camera.interrupt_at
    assert backend.get_backend_state() == "stopped"


def test_backend_serializes_concurrent_start_requests(tmp_path: Path) -> None:
    camera = SlowConnectCamera()
    backend = CameraBackendService(camera, tmp_path)
    results: list[bool] = []

    def start_backend() -> None:
        results.append(backend.start_backend())

    threads = [threading.Thread(target=start_backend) for _ in range(2)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()

    assert results == [True, True]
    assert camera.connect_calls == 1

    backend.stop_backend()


def test_stale_frame_reporting_marks_buffered_frame_not_live_when_unhealthy(
    tmp_path: Path,
) -> None:
    camera = SlowConnectCamera()
    backend = CameraBackendService(camera, tmp_path)
    backend.frame_buffer.update_frame(
        b"frame",
        {
            "timestamp": time.time(),
            "exposure_ms": 10.0,
            "capture_duration_ms": 10.0,
        },
    )

    with backend._lifecycle_lock:
        backend._health.state = BackendLifecycleState.DEGRADED

    status = _status(backend)
    assert status["capture"]["has_frame"] is True
    assert status["capture"]["frame_is_live"] is False
