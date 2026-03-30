"""Observatory Camera Backend - Clean FastAPI backend for monitoring system."""


def main() -> None:
    """Main entry point for obscam backend with graceful shutdown handling."""
    # Initialize logging first
    import os
    import sys

    from loguru import logger

    from obscam.common.logging_config import get_logger, setup_logging

    # Force clear any existing handlers to ensure clean setup
    logger.remove()

    # Check for debug mode from environment - properly working now
    debug_mode = os.getenv("OBSCAM_DEBUG", "false").lower() == "true"
    setup_logging(debug_mode=debug_mode)

    logger = get_logger("main")
    logger.info(f"Starting ObsCam with debug_mode={debug_mode}")

    # Import start_server AFTER logging is configured
    from obscam.api.main import start_server

    # Let uvicorn handle signals, we just call start_server
    try:
        start_server()
    except KeyboardInterrupt:
        # This should rarely be reached as uvicorn handles the signal
        logger.info("Keyboard interrupt received")
        sys.exit(0)
    except Exception as e:
        logger.error(f"Unexpected error: {e}")
        sys.exit(1)
