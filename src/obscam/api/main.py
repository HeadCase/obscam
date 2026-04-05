"""FastAPI application assembly and server entrypoint."""

from collections.abc import AsyncIterator
from contextlib import asynccontextmanager

import uvicorn
from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.staticfiles import StaticFiles

from obscam.api.routers.camera import router as camera_router
from obscam.api.routers.stream import router as stream_router
from obscam.api.routers.ui import router as ui_router
from obscam.api.runtime import (
    FrameDeliveryNotifier,
    get_app_runtime,
    initialize_runtime,
)
from obscam.common.constants import STATIC_DIR
from obscam.common.logging_config import get_logger

logger = get_logger("api_main")


@asynccontextmanager
async def obscam_lifespan(app: FastAPI) -> AsyncIterator[None]:
    """Bind async delivery state to the running FastAPI loop."""
    runtime = get_app_runtime(app)
    runtime.bind_notifier_to_current_loop()
    yield


def create_app() -> FastAPI:
    """Create and configure the FastAPI application."""
    app = FastAPI(title="ObsCam API", version="1.0.0", lifespan=obscam_lifespan)
    initialize_runtime(app)

    app.mount("/static", StaticFiles(directory=str(STATIC_DIR)), name="static")
    app.add_middleware(
        CORSMiddleware,
        allow_origins=["*"],
        allow_credentials=True,
        allow_methods=["*"],
        allow_headers=["*"],
    )

    app.include_router(ui_router)
    app.include_router(camera_router)
    app.include_router(stream_router)
    return app


app = create_app()


def run_server(port: int = 8000) -> None:
    """Run the FastAPI server with a small keepalive timeout."""
    config = uvicorn.Config(
        app,
        host="0.0.0.0",
        port=port,
        log_level="info",
        timeout_keep_alive=2,
    )
    server = uvicorn.Server(config)

    try:
        server.run()
    except KeyboardInterrupt:
        logger.info("Server interrupted by user")


def start_server() -> None:
    """Start the web server, initialize the backend, and shutdown cleanly."""
    runtime = get_app_runtime(app)
    backend = runtime.ensure_backend()

    logger.info("Starting backend service...")
    if backend.start_backend():
        logger.info("Backend service started successfully!")
        logger.info("Available endpoints:")
        logger.info("  - Main page: http://localhost:8000/")
        logger.info("  - Latest frame: http://localhost:8000/api/latest-frame")
        logger.info("  - Update settings: POST http://localhost:8000/api/settings")
        logger.info("  - Status: http://localhost:8000/api/status")
    else:
        logger.warning("Backend service failed to start. Use /api/connect to retry.")

    logger.info("Starting web server on port 8000...")
    logger.info("Press Ctrl+C to stop")

    try:
        run_server()
    finally:
        logger.info("Shutting down backend service...")
        backend.shutdown_gracefully()
        logger.info("Shutdown complete. Goodbye!")


__all__ = ["FrameDeliveryNotifier", "app", "create_app", "run_server", "start_server"]
