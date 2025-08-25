"""Observatory Camera Backend - Clean FastAPI backend for monitoring system."""


def main() -> None:
    """Main entry point for obscam backend."""
    # Initialize logging first
    import os
    from obscam.common.logging_config import setup_logging
    from obscam.api.main import start_server

    # Check for debug mode from environment
    debug_mode = os.getenv("OBSCAM_DEBUG", "false").lower() == "true"
    setup_logging(debug_mode=debug_mode)

    start_server()
