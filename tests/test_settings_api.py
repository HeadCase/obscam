import asyncio
from typing import TypedDict, cast

from fastapi import Request
from fastapi.routing import APIRoute

from obscam.api import main as api_main
from obscam.api.routers import camera as camera_routes
from obscam.api.runtime import ApiRuntimeState
from obscam.core.backend_service import BackendLifecycleState, CameraBackendService


class FakeCaptureLoop:
    def __init__(self) -> None:
        self.queued_settings: list[dict[str, object]] = []

    def update_settings(self, settings: dict[str, object]) -> None:
        self.queued_settings.append(settings)


class FakeSettingsManager:
    def __init__(self) -> None:
        self.saved_settings: list[dict[str, object]] = []

    def save_settings_async(self, settings: dict[str, object]) -> None:
        self.saved_settings.append(settings)


class FakeFrameBuffer:
    def __init__(self) -> None:
        self.on_new_frame = None

    def get_latest_snapshot(self):
        return None


class FakeBackend:
    def __init__(self, *, state: str = BackendLifecycleState.RUNNING.value) -> None:
        self.capture_loop = FakeCaptureLoop()
        self.settings_manager = FakeSettingsManager()
        self.current_settings: dict[str, object] = {"exposure_ms": 200.0, "gain": 250}
        self.frame_buffer = FakeFrameBuffer()
        self.state = state
        self.start_calls = 0
        self.stop_calls = 0
        self.recovery_requests = 0

    def get_backend_state(self) -> str:
        return self.state

    def get_current_settings(self) -> dict[str, object]:
        return self.current_settings.copy()

    def get_control_capabilities(self) -> dict[str, object]:
        return {
            "exposure_ms": {"min": 0.032, "max": 30000.0, "type": "float"},
            "gain": {"min": 0, "max": 600, "type": "int"},
        }

    def queue_settings_update(self, settings: dict[str, object]) -> None:
        self.capture_loop.update_settings(settings)
        self.settings_manager.save_settings_async({**self.current_settings, **settings})

    def start_backend(self) -> bool:
        self.start_calls += 1
        self.state = BackendLifecycleState.RUNNING.value
        return True

    def stop_backend(self) -> None:
        self.stop_calls += 1
        self.state = BackendLifecycleState.STOPPED.value

    def request_recovery(self) -> bool:
        self.recovery_requests += 1
        self.state = BackendLifecycleState.RECOVERING.value
        return True

    def get_status_snapshot(self) -> dict[str, object]:
        return {
            "backend": {
                "state": self.state,
                "last_error": None,
                "last_transition_at": 100.0,
                "consecutive_capture_failures": 0,
                "last_recovery_attempt_at": None,
            },
            "camera": {
                "connected": self.state == BackendLifecycleState.RUNNING.value,
                "status": "connected"
                if self.state == BackendLifecycleState.RUNNING.value
                else "disconnected",
                "model": "Fake Camera",
            },
            "capture": {
                "continuous_capture": self.state == BackendLifecycleState.RUNNING.value,
                "has_frame": False,
                "frame_timestamp": None,
                "frame_age_seconds": None,
                "frame_is_live": False,
            },
            "settings": {"current_settings": self.current_settings.copy()},
            "timestamp": 200.0,
        }


class FakeRequest:
    def __init__(self, payload: dict[str, object]) -> None:
        self.payload = payload

    async def json(self) -> dict[str, object]:
        return self.payload


class GainCapability(TypedDict):
    min: int
    max: int
    type: str


class BootstrapCapabilities(TypedDict):
    gain: GainCapability


class BootstrapBackendStatus(TypedDict):
    state: str


class BootstrapStatus(TypedDict):
    backend: BootstrapBackendStatus


class BootstrapPayload(TypedDict):
    stream_url: str
    telemetry_url: str
    capabilities: BootstrapCapabilities
    current_settings: dict[str, object]
    status: BootstrapStatus


def _request(payload: dict[str, object]) -> Request:
    return cast(Request, FakeRequest(payload))


def _typed_backend(fake_backend: FakeBackend) -> CameraBackendService:
    return cast(CameraBackendService, fake_backend)


def test_settings_endpoint_accepts_valid_updates() -> None:
    backend = FakeBackend()
    runtime = ApiRuntimeState()
    runtime.backend = _typed_backend(backend)

    payload = asyncio.run(
        camera_routes.update_settings(
            _request({"exposure_ms": 500.0, "gain": 300}),
            _typed_backend(backend),
            runtime,
        )
    )

    assert payload["status"] == "success"
    assert payload["applied_version"] == 1
    assert payload["current_settings"] == {"exposure_ms": 200.0, "gain": 250}
    assert backend.capture_loop.queued_settings == [{"exposure_ms": 500.0, "gain": 300}]
    assert backend.settings_manager.saved_settings == [
        {"exposure_ms": 500.0, "gain": 300}
    ]


def test_settings_endpoint_requires_running_backend() -> None:
    backend = FakeBackend(state=BackendLifecycleState.RECOVERING.value)
    runtime = ApiRuntimeState()
    runtime.backend = _typed_backend(backend)

    try:
        asyncio.run(
            camera_routes.update_settings(
                _request({"exposure_ms": 500.0}),
                _typed_backend(backend),
                runtime,
            )
        )
    except Exception as exc:
        assert getattr(exc, "status_code", None) == 400
        assert getattr(exc, "detail", None) == "Backend service not running"
    else:
        raise AssertionError("Expected update_settings to fail while recovering")


def test_settings_endpoint_rejects_invalid_values() -> None:
    backend = FakeBackend()
    runtime = ApiRuntimeState()
    runtime.backend = _typed_backend(backend)

    try:
        asyncio.run(
            camera_routes.update_settings(
                _request({"exposure_ms": 50001.0}),
                _typed_backend(backend),
                runtime,
            )
        )
    except Exception as exc:
        assert getattr(exc, "status_code", None) == 400
        assert getattr(exc, "detail", None) == "No valid settings provided"
    else:
        raise AssertionError("Expected invalid settings payload to fail")

    assert backend.capture_loop.queued_settings == []
    assert backend.settings_manager.saved_settings == []


def test_settings_endpoint_rejects_empty_payload() -> None:
    backend = FakeBackend()
    runtime = ApiRuntimeState()
    runtime.backend = _typed_backend(backend)

    try:
        asyncio.run(
            camera_routes.update_settings(
                _request({}),
                _typed_backend(backend),
                runtime,
            )
        )
    except Exception as exc:
        assert getattr(exc, "status_code", None) == 400
        assert getattr(exc, "detail", None) == "No valid settings provided"
    else:
        raise AssertionError("Expected empty settings payload to fail")


def test_bootstrap_endpoint_is_read_only_and_returns_status() -> None:
    backend = FakeBackend(state=BackendLifecycleState.STOPPED.value)

    payload = cast(
        BootstrapPayload,
        asyncio.run(camera_routes.bootstrap(_typed_backend(backend))),
    )

    assert payload["stream_url"] == "/stream.mjpg"
    assert payload["telemetry_url"] == "/api/telemetry"
    assert payload["capabilities"]["gain"]["max"] == 600
    assert payload["current_settings"] == {"exposure_ms": 200.0, "gain": 250}
    assert payload["status"]["backend"]["state"] == "stopped"
    assert backend.start_calls == 0


def test_backend_start_endpoint_starts_when_stopped() -> None:
    backend = FakeBackend(state=BackendLifecycleState.STOPPED.value)

    payload = asyncio.run(camera_routes.start_backend(_typed_backend(backend)))

    assert payload["backend_state"] == "running"
    assert payload["detail"] == "Backend started"
    assert backend.start_calls == 1


def test_backend_recover_endpoint_rejects_stopped_backend() -> None:
    backend = FakeBackend(state=BackendLifecycleState.STOPPED.value)

    try:
        asyncio.run(camera_routes.recover_backend(_typed_backend(backend)))
    except Exception as exc:
        assert getattr(exc, "status_code", None) == 409
        assert getattr(exc, "detail", None) == "Cannot recover backend while stopped"
    else:
        raise AssertionError("Expected recover to fail when stopped")


def test_backend_recover_endpoint_requests_recovery_when_degraded() -> None:
    backend = FakeBackend(state=BackendLifecycleState.DEGRADED.value)

    payload = asyncio.run(camera_routes.recover_backend(_typed_backend(backend)))

    assert payload["backend_state"] == "recovering"
    assert payload["detail"] == "Recovery requested"
    assert backend.recovery_requests == 1


def test_backend_stop_endpoint_stops_active_backend() -> None:
    backend = FakeBackend(state=BackendLifecycleState.RUNNING.value)

    payload = asyncio.run(camera_routes.stop_backend(_typed_backend(backend)))

    assert payload["backend_state"] == "stopped"
    assert payload["detail"] == "Backend stopped"
    assert backend.stop_calls == 1


def test_new_backend_routes_replace_legacy_connect_and_status_routes() -> None:
    get_routes = {
        route.path
        for route in api_main.app.routes
        if isinstance(route, APIRoute)
        if "GET" in getattr(route, "methods", set())
    }
    post_routes = {
        route.path
        for route in api_main.app.routes
        if isinstance(route, APIRoute)
        if "POST" in getattr(route, "methods", set())
    }

    assert "/api/backend/status" in get_routes
    assert "/api/status" not in get_routes
    assert "/api/connect" not in get_routes
    assert "/api/backend/start" in post_routes
    assert "/api/backend/recover" in post_routes
    assert "/api/backend/stop" in post_routes
