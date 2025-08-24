def main() -> None:
    """Main entry point for obscam."""
    # Initialize logging first
    import os
    from .logging_config import setup_logging

    # Check for debug mode from environment
    debug_mode = os.getenv("OBSCAM_DEBUG", "false").lower() == "true"
    port = int(os.getenv("OBSCAM_FLASK_PORT", "5000"))
    setup_logging(debug_mode=debug_mode)

    from .web import start_web_servers

    start_web_servers(flask_port=port)
