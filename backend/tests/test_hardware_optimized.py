#!/usr/bin/env python3
"""Optimized hardware tests for backend service - minimal logging, robust error handling."""

import os
import sys
import time

# Set environment for gphoto2 camera (development)
os.environ["OBSCAM_CAMERA_TYPE"] = "gphoto2"

sys.path.insert(0, "src")

from obscam.camera_factory import get_backend_service
from obscam.logging_config import get_logger

# Get logger - we'll minimize console output manually
logger = get_logger("test_hardware")


class TestTimeout(Exception):
    """Custom exception for test timeouts."""

    pass


def wait_with_timeout(
    condition_func, timeout_seconds: float, check_interval: float = 0.1
) -> bool:
    """Wait for a condition with timeout protection."""
    start_time = time.time()
    while time.time() - start_time < timeout_seconds:
        if condition_func():
            return True
        time.sleep(check_interval)
    return False


def test_camera_basic_connection() -> bool:
    """Test direct camera connection without full backend complexity."""
    print("🔌 Testing basic camera connection...")

    backend = None
    try:
        backend = get_backend_service()

        # Test connection only - no capture loop
        camera = backend.camera
        connected = camera.connect()

        if not connected:
            print("❌ Camera failed to connect")
            return False

        # Get basic status
        status = camera.get_status()
        print(f"✅ Camera connected: {status.get('camera_model', 'Unknown')}")

        # Test disconnect
        camera.disconnect()
        print("✅ Camera disconnected cleanly")
        return True

    except Exception as e:
        print(f"❌ Connection test failed: {e}")
        return False
    finally:
        if backend and hasattr(backend, "camera"):
            try:
                backend.camera.disconnect()
            except:
                pass


def test_backend_minimal_lifecycle() -> bool:
    """Test backend start/stop with minimal complexity."""
    print("🔄 Testing backend lifecycle...")

    backend = None
    try:
        backend = get_backend_service()

        # Test start
        success = backend.start_backend()
        if not success:
            print("❌ Backend failed to start")
            return False

        print("✅ Backend started")

        # Quick status check
        if not backend.is_started():
            print("❌ Backend not reporting as started")
            return False

        # Test stop
        backend.stop_backend()

        if backend.is_started():
            print("❌ Backend still running after stop")
            return False

        print("✅ Backend stopped cleanly")
        return True

    except Exception as e:
        print(f"❌ Lifecycle test failed: {e}")
        return False
    finally:
        if backend:
            try:
                backend.stop_backend()
            except:
                pass


def test_single_frame_capture() -> bool:
    """Test single frame capture with adaptive timing."""
    print("📸 Testing frame capture...")

    backend = None
    try:
        backend = get_backend_service()

        # Start backend
        if not backend.start_backend():
            print("❌ Backend failed to start")
            return False

        # Give capture loop time to actually start capturing
        print("⏳ Letting capture loop initialize...")
        time.sleep(3.0)  # Let capture loop start and capture at least one frame

        # Check if capture loop is actually running
        if not backend.capture_loop.is_running():
            print("❌ Capture loop is not running")
            return False

        print("⏳ Waiting for fresh frame...")

        # Wait for a fresh frame (not cached)
        def has_fresh_frame():
            frame = backend.frame_buffer.get_latest_frame()
            metadata = backend.frame_buffer.get_frame_metadata()
            return frame is not None and metadata is not None

        # Allow up to 15 seconds for first frame (longer for DSLR)
        if not wait_with_timeout(has_fresh_frame, timeout_seconds=15.0):
            print("❌ No fresh frame captured within timeout")
            # No frame captured within timeout
            return False

        frame_data = backend.frame_buffer.get_latest_frame()
        metadata = backend.frame_buffer.get_frame_metadata()

        # Basic validation
        if len(frame_data) < 1000:
            print(f"❌ Frame too small: {len(frame_data)} bytes")
            return False

        # JPEG validation
        if not (
            frame_data.startswith(b"\xff\xd8") and frame_data.endswith(b"\xff\xd9")
        ):
            print("❌ Invalid JPEG format")
            return False

        if not metadata or "exposure_ms" not in metadata:
            print("❌ Missing frame metadata")
            return False

        print(
            f"✅ Fresh frame captured: {len(frame_data)} bytes, {metadata['exposure_ms']}ms"
        )
        return True

    except Exception as e:
        print(f"❌ Frame capture test failed: {e}")
        return False
    finally:
        if backend:
            try:
                backend.stop_backend()
            except:
                pass


def test_simple_exposure_change() -> bool:
    """Test single exposure setting change with validation."""
    print("⚙️  Testing exposure setting change...")

    backend = None
    try:
        backend = get_backend_service()

        if not backend.start_backend():
            print("❌ Backend failed to start")
            return False

        # Give capture loop time to start
        time.sleep(2.0)

        # Check if capture loop is running
        if not backend.capture_loop.is_running():
            print("❌ Capture loop is not running")
            return False

        # Get initial settings
        initial_settings = backend.get_current_settings()
        initial_exposure = initial_settings.get("exposure_ms", 0)

        # Choose a shorter exposure for faster testing (avoid long DSLR exposures)
        test_exposure = 500.0 if initial_exposure != 500.0 else 250.0
        print(f"⏳ Changing exposure: {initial_exposure}ms → {test_exposure}ms")

        # Update settings
        if not backend.update_settings(exposure_ms=test_exposure):
            print("❌ Failed to update settings")
            return False

        # Verify settings were applied to camera
        current_settings = backend.get_current_settings()
        if current_settings.get("exposure_ms") != test_exposure:
            print(
                f"❌ Exposure not applied to camera: {current_settings.get('exposure_ms')} != {test_exposure}"
            )
            return False

        # Wait longer for DSLR frame capture and settings to take effect
        print("⏳ Waiting for new frame with updated exposure...")
        time.sleep(max(test_exposure / 1000.0 + 3.0, 5.0))  # Longer wait for DSLR

        def has_new_frame_with_exposure():
            # Check frame buffer directly for fresh frame with correct exposure
            metadata = backend.frame_buffer.get_frame_metadata()
            return metadata and metadata.get("exposure_ms") == test_exposure

        # Allow more time for DSLR to capture with new settings
        timeout = max(test_exposure / 1000.0 * 3, 10.0)
        if not wait_with_timeout(has_new_frame_with_exposure, timeout_seconds=timeout):
            print("❌ New frame with updated exposure not captured")

            # Check what metadata we have
            current_metadata = backend.frame_buffer.get_frame_metadata()
            if current_metadata:
                current_exp = current_metadata.get("exposure_ms", "unknown")
                print(f"ℹ️  Current frame metadata exposure: {current_exp}ms")
            else:
                print("ℹ️  No metadata available")
            return False

        final_metadata = backend.frame_buffer.get_frame_metadata()
        captured_exposure = final_metadata["exposure_ms"]

        if abs(captured_exposure - test_exposure) > 1.0:  # 1ms tolerance
            print(f"❌ Exposure mismatch: {captured_exposure} != {test_exposure}")
            return False

        print(f"✅ Exposure change successful: {captured_exposure}ms")
        return True

    except Exception as e:
        print(f"❌ Exposure test failed: {e}")
        return False
    finally:
        if backend:
            try:
                backend.stop_backend()
            except:
                pass


def test_error_recovery() -> bool:
    """Test basic error recovery scenarios."""
    print("🛠️  Testing error recovery...")

    backend = None
    try:
        backend = get_backend_service()

        # Test double start (should be safe)
        backend.start_backend()
        result = backend.start_backend()  # Should return True but not crash

        if not result:
            print("❌ Double start failed")
            return False

        # Test double stop (should be safe)
        backend.stop_backend()
        backend.stop_backend()  # Should not crash

        if backend.is_started():
            print("❌ Backend still running after double stop")
            return False

        print("✅ Error recovery tests passed")
        return True

    except Exception as e:
        print(f"❌ Error recovery test failed: {e}")
        return False
    finally:
        if backend:
            try:
                backend.stop_backend()
            except:
                pass


def main():
    """Run optimized hardware tests."""
    print("🚀 Starting optimized hardware tests...\n")

    tests = [
        ("Camera Connection", test_camera_basic_connection),
        ("Backend Lifecycle", test_backend_minimal_lifecycle),
        ("Frame Capture", test_single_frame_capture),
        ("Exposure Settings", test_simple_exposure_change),
        ("Error Recovery", test_error_recovery),
    ]

    passed = 0
    failed = 0

    for test_name, test_func in tests:
        print(f"\n{'='*50}")
        print(f"Running: {test_name}")
        print("=" * 50)

        try:
            start_time = time.time()
            result = test_func()
            duration = time.time() - start_time

            if result:
                passed += 1
                print(f"✅ {test_name} PASSED ({duration:.1f}s)")
            else:
                failed += 1
                print(f"❌ {test_name} FAILED ({duration:.1f}s)")

        except Exception as e:
            failed += 1
            print(f"❌ {test_name} CRASHED: {e}")

        # Brief pause between tests for cleanup
        time.sleep(0.5)

    print(f"\n{'='*50}")
    print(f"🎯 Results: {passed} passed, {failed} failed")
    print("=" * 50)

    if failed == 0:
        print("🎉 All tests passed!")
        return 0
    else:
        print("💥 Some tests failed!")
        return 1


if __name__ == "__main__":
    sys.exit(main())
