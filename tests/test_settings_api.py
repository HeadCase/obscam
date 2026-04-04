import asyncio
from typing import Any, cast

from fastapi import Request
from fastapi.routing import APIRoute

from obscam.api import main as api_main


class FakeCaptureLoop:
    def __init__(self) -> None:
        self.queued_settings: list[dict[str, Any]] = []

    def update_settings(self, settings: dict[str, Any]) -> None:
        self.queued_settings.append(settings)


class FakeSettingsManager:
    def __init__(self) -> None:
        self.saved_settings: list[dict[str, Any]] = []

    def save_settings_async(self, settings: dict[str, Any]) -> None:
        self.saved_settings.append(settings)


class FakeCamera:
    def get_control_capabilities(self) -> dict[str, object]:
        return {
            "exposure_ms": {"min": 0.032, "max": 30000.0, "type": "float"},
            "gain": {"min": 0, "max": 600, "type": "int"},
        }


class FakeBackend:
    def __init__(self) -> None:
        self.camera = FakeCamera()
        self.capture_loop = FakeCaptureLoop()
        self.settings_manager = FakeSettingsManager()
        self.current_settings: dict[str, Any] = {"exposure_ms": 200.0, "gain": 250}

    def is_started(self) -> bool:
        return True

    def get_current_settings(self) -> dict[str, Any]:
        return self.current_settings.copy()


class FakeRequest:
    def __init__(self, payload: dict[str, Any]) -> None:
        self.payload = payload

    async def json(self) -> dict[str, Any]:
        return self.payload


def _request(payload: dict[str, Any]) -> Request:
    return cast(Request, FakeRequest(payload))


def test_settings_endpoint_accepts_valid_updates(monkeypatch) -> None:
    backend = FakeBackend()
    monkeypatch.setattr(api_main, "backend", backend)
    monkeypatch.setattr(api_main, "settings_version", 0)

    payload = asyncio.run(
        api_main.update_settings_v2(_request({"exposure_ms": 500.0, "gain": 300}))
    )

    assert payload["status"] == "success"
    assert payload["applied_version"] == 1
    assert payload["current_settings"] == {"exposure_ms": 200.0, "gain": 250}
    assert backend.capture_loop.queued_settings == [{"exposure_ms": 500.0, "gain": 300}]
    assert backend.settings_manager.saved_settings == [
        {"exposure_ms": 500.0, "gain": 300}
    ]


def test_settings_endpoint_rejects_invalid_values(monkeypatch) -> None:
    backend = FakeBackend()
    monkeypatch.setattr(api_main, "backend", backend)

    try:
        asyncio.run(api_main.update_settings_v2(_request({"exposure_ms": 50001.0})))
    except Exception as exc:
        assert getattr(exc, "status_code", None) == 400
        assert getattr(exc, "detail", None) == "No valid settings provided"
    else:
        raise AssertionError("Expected invalid settings payload to fail")

    assert backend.capture_loop.queued_settings == []
    assert backend.settings_manager.saved_settings == []


def test_settings_endpoint_rejects_empty_payload(monkeypatch) -> None:
    backend = FakeBackend()
    monkeypatch.setattr(api_main, "backend", backend)

    try:
        asyncio.run(api_main.update_settings_v2(_request({})))
    except Exception as exc:
        assert getattr(exc, "status_code", None) == 400
        assert getattr(exc, "detail", None) == "No valid settings provided"
    else:
        raise AssertionError("Expected empty settings payload to fail")


def test_legacy_update_settings_endpoint_is_removed() -> None:
    post_routes = {
        route.path
        for route in api_main.app.routes
        if isinstance(route, APIRoute)
        if "POST" in getattr(route, "methods", set())
    }

    assert "/api/settings" in post_routes
    assert "/api/update-settings" not in post_routes
