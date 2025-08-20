def main() -> None:
    """Main entry point for obscam."""
    from .web import start_web_servers
    start_web_servers()
