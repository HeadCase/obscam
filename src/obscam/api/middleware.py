"""API middleware configuration."""

from fastapi.middleware.cors import CORSMiddleware


def setup_cors_middleware(app):
    """Setup CORS middleware for cross-origin requests."""
    app.add_middleware(
        CORSMiddleware,
        allow_origins=["*"],
        allow_credentials=True,
        allow_methods=["*"],
        allow_headers=["*"],
    )
