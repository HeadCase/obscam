"""FastAPI control and JPEG delivery surface for GRE-190."""

from __future__ import annotations

import asyncio
import json
import struct
import time
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from fastapi.responses import HTMLResponse, StreamingResponse

from obscam.tools.gre_190_prototype.contract import (
    BrowserPresentation,
    BrowserRunReport,
)
from obscam.tools.gre_190_prototype.latest import LatestFrameFanout
from obscam.tools.gre_190_prototype.preflight import inspect_capabilities
from obscam.tools.gre_190_prototype.source import GeneratedJpegSource

BOUNDARY = b"gre190frame"


def create_app(*, fps: float = 10, quality: int = 80) -> FastAPI:
    """Create an isolated generated-source benchmark application."""
    fanout = LatestFrameFanout()
    source = GeneratedJpegSource(fanout, fps=fps, quality=quality)
    presentations: list[BrowserPresentation] = []
    completed_runs: dict[str, dict[str, BrowserRunReport]] = {}
    counters = {"websocket_delivered": 0, "mjpeg_delivered": 0}

    @asynccontextmanager
    async def lifespan(_app: FastAPI) -> AsyncIterator[None]:
        task = asyncio.create_task(source.run())
        try:
            yield
        finally:
            task.cancel()
            await asyncio.gather(task, return_exceptions=True)

    app = FastAPI(title="GRE-190 throwaway benchmark", lifespan=lifespan)

    @app.get("/", response_class=HTMLResponse)
    async def index() -> str:
        return Path(__file__).with_name("client.html").read_text()

    @app.get("/api/preflight")
    async def preflight() -> dict[str, object]:
        return json.loads(inspect_capabilities().to_json())

    @app.get("/api/status")
    async def status() -> dict[str, object]:
        return {
            "produced": source.counters.produced,
            "encoded_bytes": source.counters.encoded_bytes,
            "presentations": len(presentations),
            "completed_runs": sum(len(clients) for clients in completed_runs.values()),
            **counters,
        }

    @app.get("/api/clock")
    async def clock() -> dict[str, int]:
        server_receive_unix_ns = time.time_ns()
        return {
            "server_receive_unix_ns": server_receive_unix_ns,
            "server_send_unix_ns": time.time_ns(),
        }

    @app.post("/api/presentations", status_code=204)
    async def record_presentation(event: BrowserPresentation) -> None:
        presentations.append(event)

    @app.post("/api/runs", status_code=201)
    async def record_run(report: BrowserRunReport) -> dict[str, object]:
        completed_runs.setdefault(report.run_id, {})[report.client_id] = report
        presentations.extend(report.presentations)
        return {
            "run_id": report.run_id,
            "client_id": report.client_id,
            "stored": True,
        }

    @app.get("/api/runs/{run_id}")
    async def get_run(run_id: str) -> dict[str, object]:
        clients = completed_runs.get(run_id, {})
        return {
            "run_id": run_id,
            "complete_clients": sorted(clients),
            "reports": {
                client_id: report.model_dump(mode="json")
                for client_id, report in clients.items()
            },
        }

    @app.websocket("/ws/jpeg")
    async def jpeg_websocket(websocket: WebSocket) -> None:
        await websocket.accept()
        generation = 0
        try:
            while True:
                frame = await fanout.wait_after(generation)
                generation = frame.envelope.generation
                header = frame.envelope.model_dump_json().encode()
                message = struct.pack("!I", len(header)) + header + frame.payload
                await websocket.send_bytes(message)
                counters["websocket_delivered"] += 1
        except WebSocketDisconnect:
            return

    async def mjpeg_stream() -> AsyncIterator[bytes]:
        generation = 0
        while True:
            frame = await fanout.wait_after(generation)
            generation = frame.envelope.generation
            metadata = frame.envelope.model_dump_json()
            counters["mjpeg_delivered"] += 1
            yield (
                b"--"
                + BOUNDARY
                + b"\r\nContent-Type: image/jpeg\r\n"
                + f"Content-Length: {len(frame.payload)}\r\n".encode()
                + f"X-GRE190-Frame: {metadata}\r\n\r\n".encode()
                + frame.payload
                + b"\r\n"
            )

    @app.get("/mjpeg")
    async def mjpeg() -> StreamingResponse:
        return StreamingResponse(
            mjpeg_stream(),
            media_type=f"multipart/x-mixed-replace; boundary={BOUNDARY.decode()}",
        )

    return app
