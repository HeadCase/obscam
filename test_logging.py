#!/usr/bin/env python3
"""Test script for ObsCam logging implementation."""

import tempfile
from pathlib import Path


def test_logging_configuration():
    """Test the logging configuration and output."""
    print("🔧 Testing ObsCam Logging Implementation")
    print("=" * 50)

    # Create a temporary directory for logs
    with tempfile.TemporaryDirectory() as temp_dir:
        temp_log_dir = Path(temp_dir) / "test_logs"

        try:
            # Import and setup logging with custom directory
            from src.obscam.logging_config import (
                setup_logging,
                get_logger,
                log_camera_event,
                log_session_event,
                log_websocket_event,
            )

            setup_logging(debug_mode=True, log_dir_str=str(temp_log_dir))

            print("✓ Logging setup successful")
            print(f"  Log directory: {temp_log_dir}")

            # Test basic logging
            logger = get_logger("test_module")
            logger.info("Test info message", test_param="value")
            logger.warning("Test warning message", param1="test", param2=123)
            logger.error("Test error message", error="simulated error")

            print("✓ Basic logging operations successful")

            # Test structured logging functions
            log_camera_event(
                "test_connection", camera="Test Camera", status="connected"
            )
            log_session_event(
                "test_session", "test_session_123", ip_address="192.168.1.100"
            )
            log_websocket_event("test_connect", client_id="test_client")

            print("✓ Structured logging functions successful")

            # Check log files were created
            expected_files = ["obscam.log", "obscam-error.log", "obscam-debug.log"]
            created_files = []

            for file_name in expected_files:
                log_file = temp_log_dir / file_name
                if log_file.exists() and log_file.stat().st_size > 0:
                    created_files.append(file_name)
                    with open(log_file, "r") as f:
                        content = f.read()
                        lines = len(content.strip().split("\n"))
                        print(
                            f"  ✓ {file_name}: {lines} log entries, {log_file.stat().st_size} bytes"
                        )

            if len(created_files) == len(expected_files):
                print("✓ All expected log files created successfully")
                return True
            else:
                missing = set(expected_files) - set(created_files)
                print(f"✗ Missing log files: {missing}")
                return False

        except ImportError as e:
            print(f"✗ Import error: {e}")
            print(
                "  Note: This is expected until dependencies are installed with 'uv sync'"
            )
            return False
        except Exception as e:
            print(f"✗ Logging test failed: {e}")
            return False


def test_pi_compatibility():
    """Test Pi-specific logging features."""
    print("\n🔧 Testing Pi Environment Compatibility")
    print("=" * 50)

    try:
        # Test log directory creation in typical Pi locations
        pi_home = Path.home()
        test_log_dir = pi_home / "test-obscam-logs"

        # Clean up from any previous test
        if test_log_dir.exists():
            import shutil

            shutil.rmtree(test_log_dir)

        # Test directory creation
        test_log_dir.mkdir(exist_ok=True)

        if test_log_dir.exists():
            print(f"✓ Log directory creation successful: {test_log_dir}")

            # Clean up
            test_log_dir.rmdir()
            print("✓ Log directory cleanup successful")
            return True
        else:
            print("✗ Failed to create log directory")
            return False

    except Exception as e:
        print(f"✗ Pi compatibility test failed: {e}")
        return False


def show_usage_instructions():
    """Show usage instructions for production deployment."""
    print("\n📋 Production Deployment Instructions")
    print("=" * 50)

    print("""
To use robust logging in production:

1. **Install dependencies:**
   uv sync

2. **Enable debug mode (optional):**
   export OBSCAM_DEBUG=true

3. **Run application:**
   uv run obscam

4. **Access logs:**
   - Main logs: ~/obscam-logs/obscam.log
   - Error logs: ~/obscam-logs/obscam-error.log  
   - Debug logs: ~/obscam-logs/obscam-debug.log (debug mode only)

5. **Monitor logs remotely:**
   - SSH: ssh pi@your-observatory "tail -f ~/obscam-logs/obscam.log"
   - SCP: scp pi@your-observatory:~/obscam-logs/*.log ./local-logs/

6. **Log rotation:**
   - Logs automatically rotate at 10MB
   - Main/error logs kept for 7-14 days
   - Debug logs kept for 3 days
   - Compression enabled to save space

Key features:
✓ Pi-optimized file sizes and retention
✓ Structured logging for easy parsing
✓ USB camera recovery event logging
✓ Session management event logging  
✓ WebSocket error debugging
✓ Performance metrics logging
✓ Automatic log rotation and compression
""")


if __name__ == "__main__":
    print("ObsCam Logging Implementation Test")
    print("=" * 50)

    success_count = 0
    total_tests = 2

    # Test 1: Logging configuration
    if test_logging_configuration():
        success_count += 1

    # Test 2: Pi compatibility
    if test_pi_compatibility():
        success_count += 1

    # Show results
    print(f"\n🎯 Test Results: {success_count}/{total_tests} passed")

    if success_count == total_tests:
        print("🎉 All logging tests passed! Ready for production deployment.")
    else:
        print("⚠️  Some tests failed. Install dependencies with 'uv sync' and retry.")

    # Always show usage instructions
    show_usage_instructions()
