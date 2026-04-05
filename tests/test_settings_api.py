import asyncio
from typing import TypedDict, cast

from fastapi import Request
from fastapi.routing import APIRoute

from obscam.api import main as api_main
from obscam.api.routers import camera as camera_routes
from obscam.api.runtime import ApiRuntimeState
from obscam.core.backend_service import CameraBackendService


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
    def __init__(self) -> None:
        self.capture_loop = FakeCaptureLoop()
        self.settings_manager = FakeSettingsManager()
        self.current_settings: dict[str, object] = {"exposure_ms": 200.0, "gain": 250}
        self.frame_buffer = FakeFrameBuffer()
        self.started = True
        self.start_calls = 0

    def is_started(self) -> bool:
        return self.started

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
        self.started = True
        return True

    def get_status(self) -> dict[str, object]:
        return {"backend_service": "running", "has_frame": False}


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


class BootstrapPayload(TypedDict):
    stream_url: str
    telemetry_url: str
    capabilities: BootstrapCapabilities


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


def test_bootstrap_endpoint_returns_expected_urls() -> None:
    backend = FakeBackend()

    payload = cast(
        BootstrapPayload,
        asyncio.run(camera_routes.bootstrap(_typed_backend(backend))),
    )

    assert payload["stream_url"] == "/stream.mjpg"
    assert payload["telemetry_url"] == "/api/telemetry"
    assert payload["capabilities"]["gain"]["max"] == 600


def test_legacy_update_settings_endpoint_is_removed() -> None:
    post_routes = {
        route.path
        for route in api_main.app.routes
        if isinstance(route, APIRoute)
        if "POST" in getattr(route, "methods", set())
    }

    assert "/api/settings" in post_routes
    assert "/api/update-settings" not in post_routes
