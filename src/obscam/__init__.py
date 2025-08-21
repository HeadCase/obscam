def main() -> None:
    """Main entry point for obscam."""
    # Initialize logging first
    import os
    from .logging_config import setup_logging

    # Check for debug mode from environment
    debug_mode = os.getenv("OBSCAM_DEBUG", "false").lower() == "true"
    setup_logging(debug_mode=debug_mode)

    from .web import start_web_servers

    start_web_servers()
