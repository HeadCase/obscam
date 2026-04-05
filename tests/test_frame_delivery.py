import asyncio
import json
from collections.abc import AsyncGenerator
from pathlib import Path
from typing import cast

from obscam.api.routers import camera as camera_routes
from obscam.api.routers import stream as stream_routes
from obscam.api.runtime import ApiRuntimeState, FrameDeliveryNotifier
from obscam.core.backend_service import CameraBackendService
from obscam.core.frame_buffer import LatestFrameBuffer


class DummyCamera:
    def connect(self) -> bool:
        return True

    def disconnect(self) -> None:
        return None

    def get_status(self) -> dict[str, object]:
        return {"status": "connected"}

    def capture_frame(self) -> bytes | None:
        return b"dummy-frame"

    def update_settings(self, **settings: object) -> bool:
        return True

    def get_current_settings(self) -> dict[str, object]:
        return {"exposure_ms": 200.0, "gain": 250}

    def get_control_capabilities(self) -> dict[str, object]:
        return {
            "exposure_ms": {"min": 0.032, "max": 30000.0, "type": "float"},
            "gain": {"min": 0, "max": 600, "type": "int"},
        }


def test_frame_buffer_returns_latest_snapshot_without_consuming() -> None:
    buffer = LatestFrameBuffer()

    buffer.update_frame(
        b"first-frame",
        {"timestamp": 1.0, "exposure_ms": 100.0, "capture_duration_ms": 100.0},
    )
    buffer.update_frame(
        b"second-frame",
        {"timestamp": 2.0, "exposure_ms": 200.0, "capture_duration_ms": 200.0},
    )

    snapshot = buffer.get_latest_snapshot()

    assert snapshot is not None
    assert snapshot.frame_bytes == b"second-frame"
    assert snapshot.generation == 2
    assert snapshot.metadata["timestamp"] == 2.0
    assert buffer.get_latest_frame() == b"second-frame"
    assert buffer.get_latest_frame() == b"second-frame"
    assert buffer.get_frame_metadata() == {
        "timestamp": 2.0,
        "exposure_ms": 200.0,
        "capture_duration_ms": 200.0,
    }
    assert buffer.get_frame_with_metadata() == (
        b"second-frame",
        {
            "timestamp": 2.0,
            "exposure_ms": 200.0,
            "capture_duration_ms": 200.0,
        },
    )


def test_frame_buffer_callback_receives_monotonic_generations() -> None:
    buffer = LatestFrameBuffer()
    seen_generations: list[int] = []
    buffer.on_new_frame = lambda snapshot: seen_generations.append(snapshot.generation)

    buffer.update_frame(b"frame-a", {"timestamp": 1.0})
    buffer.update_frame(b"frame-b", {"timestamp": 2.0})

    assert seen_generations == [1, 2]


def test_delivery_notifier_wakes_on_new_frame() -> None:
    async def scenario() -> None:
        notifier = FrameDeliveryNotifier()
        buffer = LatestFrameBuffer()
        notifier.bind(
            asyncio.get_running_loop(),
            frame_generation=0,
            current_settings_version=0,
        )
        buffer.on_new_frame = notifier.publish_frame

        waiter = asyncio.create_task(notifier.wait_for_frame(0))
        buffer.update_frame(b"frame-1", {"timestamp": 1.0})

        generation = await asyncio.wait_for(waiter, timeout=1.0)
        assert generation == 1

    asyncio.run(scenario())


def test_delivery_notifier_keeps_latest_generation_when_frames_arrive_quickly() -> None:
    async def scenario() -> None:
        notifier = FrameDeliveryNotifier()
        buffer = LatestFrameBuffer()
        notifier.bind(
            asyncio.get_running_loop(),
            frame_generation=0,
            current_settings_version=0,
        )
        buffer.on_new_frame = notifier.publish_frame

        buffer.update_frame(b"frame-1", {"timestamp": 1.0})
        await asyncio.wait_for(notifier.wait_for_frame(0), timeout=1.0)
        buffer.update_frame(b"frame-2", {"timestamp": 2.0})
        buffer.update_frame(b"frame-3", {"timestamp": 3.0})
        await asyncio.sleep(0.01)

        generation = await asyncio.wait_for(notifier.wait_for_frame(1), timeout=1.0)
        assert generation == 3

    asyncio.run(scenario())


def test_delivery_notifier_wakes_on_settings_update_before_next_frame() -> None:
    async def scenario() -> None:
        notifier = FrameDeliveryNotifier()
        notifier.bind(
            asyncio.get_running_loop(),
            frame_generation=0,
            current_settings_version=0,
        )

        waiter = asyncio.create_task(notifier.wait_for_update(0, 0))
        notifier.publish_settings(1)

        frame_generation, current_settings_version = await asyncio.wait_for(
            waiter, timeout=1.0
        )
        assert frame_generation == 0
        assert current_settings_version == 1

    asyncio.run(scenario())


def test_latest_frame_response_uses_one_atomic_snapshot(
    tmp_path: Path,
) -> None:
    service = CameraBackendService(DummyCamera(), tmp_path)
    service._started = True
    service.frame_buffer.update_frame(
        b"first-frame",
        {"timestamp": 1.0, "exposure_ms": 100.0, "capture_duration_ms": 100.0},
    )
    service.frame_buffer.update_frame(
        b"latest-frame",
        {"timestamp": 5.0, "exposure_ms": 250.0, "capture_duration_ms": 250.0},
    )

    response = asyncio.run(camera_routes.get_latest_frame(service))

    assert response.body == b"latest-frame"
    assert response.headers["x-frame-timestamp"] == "5.0"
    assert response.headers["x-frame-age-seconds"]


def test_telemetry_initial_snapshot_uses_existing_frame(tmp_path: Path) -> None:
    async def scenario() -> None:
        service = CameraBackendService(DummyCamera(), tmp_path)
        service._started = True
        service.frame_buffer.update_frame(
            b"latest-frame",
            {"timestamp": 7.0, "exposure_ms": 300.0, "capture_duration_ms": 300.0},
        )
        runtime = ApiRuntimeState()
        runtime.backend = service
        runtime.settings_version = 4

        response = await stream_routes.telemetry_sse(service, runtime)
        body_iterator = cast(
            AsyncGenerator[str, None],
            response.body_iterator,
        )
        first_chunk = await anext(body_iterator)
        await body_iterator.aclose()

        payload = json.loads(first_chunk.split("data: ", maxsplit=1)[1])
        assert payload["has_frame"] is True
        assert payload["timestamp"] == 7.0
        assert payload["capture_ms"] == 300.0
        assert "settings_version" not in payload

    asyncio.run(scenario())
