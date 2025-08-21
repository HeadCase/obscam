import os
import sys
from pathlib import Path
from loguru import logger


def setup_logging(debug_mode: bool = False, log_dir_str: str | None = None) -> None:
    """
    Configure loguru for ObsCam with Pi-optimized settings.

    Args:
        debug_mode: Enable verbose debug logging
        log_dir_str: Custom log directory (defaults to ~/obscam-logs)
    """
    # Remove default handler
    logger.remove()

    # Determine log directory
    if log_dir_str is None:
        # Default to home directory for easy access via SSH
        home_dir = Path.home()
        log_dir = home_dir / "obscam-logs"
    else:
        log_dir = Path(log_dir_str)

    # Create log directory if it doesn't exist
    log_dir.mkdir(exist_ok=True)

    # Console logging with colors (INFO level for production)
    console_level = "DEBUG" if debug_mode else "INFO"
    logger.add(
        sys.stdout,
        level=console_level,
        format="<green>{time:YYYY-MM-DD HH:mm:ss}</green> | <level>{level: <8}</level> | <cyan>{name}</cyan>:<cyan>{function}</cyan>:<cyan>{line}</cyan> - <level>{message}</level>",
        colorize=True,
    )

    # Main application log - INFO level and above
    logger.add(
        log_dir / "obscam.log",
        level="INFO",
        rotation="10 MB",  # Pi-friendly file sizes
        retention="7 days",  # Keep a week for debugging
        compression="gz",  # Save disk space
        format="{time:YYYY-MM-DD HH:mm:ss.SSS} | {level: <8} | {name}:{function}:{line} | {extra} | {message}",
        serialize=False,
    )

    # Error-only log for quick issue identification
    logger.add(
        log_dir / "obscam-error.log",
        level="ERROR",
        rotation="5 MB",
        retention="14 days",  # Keep errors longer
        compression="gz",
        format="{time:YYYY-MM-DD HH:mm:ss.SSS} | {level: <8} | {name}:{function}:{line} | {extra} | {message}",
        serialize=False,
    )

    # Debug log (only in debug mode)
    if debug_mode:
        logger.add(
            log_dir / "obscam-debug.log",
            level="DEBUG",
            rotation="20 MB",  # Larger for verbose debug info
            retention="3 days",  # Shorter retention for debug logs
            compression="gz",
            format="{time:YYYY-MM-DD HH:mm:ss.SSS} | {level: <8} | {name}:{function}:{line} | {extra} | {message}",
            serialize=False,
        )

    # Log the setup
    logger.info(
        "Logging initialized",
        log_dir=str(log_dir),
        debug_mode=debug_mode,
        console_level=console_level,
    )


def get_logger(name: str) -> "logger":
    """
    Get a logger instance with structured context.

    Args:
        name: Module name (e.g., 'camera', 'session_manager', 'web')

    Returns:
        Configured logger instance
    """
    return logger.bind(module=name)


def log_performance(func_name: str, duration_ms: float, **kwargs) -> None:
    """
    Log performance metrics for critical operations.

    Args:
        func_name: Name of the function being measured
        duration_ms: Duration in milliseconds
        **kwargs: Additional context (e.g., frame_size, client_count)
    """
    logger.info(
        "Performance metric",
        function=func_name,
        duration_ms=round(duration_ms, 2),
        **kwargs,
    )


def log_camera_event(event_type: str, **kwargs) -> None:
    """
    Log camera-specific events with structured data.

    Args:
        event_type: Type of event ('connection', 'capture', 'usb_recovery', etc.)
        **kwargs: Event-specific data
    """
    logger.info("Camera event", event_type=event_type, **kwargs)


def log_session_event(event_type: str, session_id: str, **kwargs) -> None:
    """
    Log session management events.

    Args:
        event_type: Type of event ('created', 'expired', 'master_transfer', etc.)
        session_id: Session identifier
        **kwargs: Additional session data
    """
    logger.info(
        "Session event",
        event_type=event_type,
        session_id=session_id[:8],  # Truncate for privacy
        **kwargs,
    )


def log_websocket_event(event_type: str, **kwargs) -> None:
    """
    Log WebSocket events for connection debugging.

    Args:
        event_type: Type of event ('connect', 'disconnect', 'error', 'broadcast')
        **kwargs: Event-specific data
    """
    logger.info("WebSocket event", event_type=event_type, **kwargs)


# Environment-based debug mode detection
DEBUG_MODE = os.getenv("OBSCAM_DEBUG", "false").lower() == "true"

# Initialize logging when module is imported
if not logger._core.handlers:  # Avoid double initialization
    setup_logging(debug_mode=DEBUG_MODE)
