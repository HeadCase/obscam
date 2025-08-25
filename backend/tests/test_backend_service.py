#!/usr/bin/env python3
"""Test script for backend service architecture."""

import os
import sys
import time
from pathlib import Path

# Set environment for gphoto2 camera (development)
os.environ["OBSCAM_CAMERA_TYPE"] = "gphoto2"

sys.path.insert(0, "src")

from obscam.camera_factory import get_backend_service
from obscam.logging_config import get_logger

logger = get_logger("test_backend")


def test_capture_loop_start_stop():
    """Test basic capture loop start/stop functionality."""
    logger.info("=== Testing Capture Loop Start/Stop ===")

    backend = get_backend_service()

    # Verify initial state
    assert not backend.is_started(), "Backend should not be started initially"

    # Start backend service
    logger.info("Starting backend service...")
    success = backend.start_backend()
    assert success, "Backend service should start successfully"
    assert backend.is_started(), "Backend should be marked as started"

    # Check status
    status = backend.get_status()
    logger.info("Backend status", status=status)
    assert status["backend_service"] == "running", "Backend service should be running"
    assert status.get(
        "continuous_capture", False
    ), "Continuous capture should be running"

    # Wait a moment for capture loop to initialize
    time.sleep(2)

    # Stop backend service
    logger.info("Stopping backend service...")
    backend.stop_backend()
    assert not backend.is_started(), "Backend should not be started after stop"

    final_status = backend.get_status()
    assert (
        final_status["backend_service"] == "stopped"
    ), "Backend service should be stopped"

    logger.info("✅ Capture loop start/stop test passed")
    return True


def test_frame_retrieval_and_rendering():
    """Test frame retrieval and verify it's a valid JPEG image."""
    logger.info("=== Testing Frame Retrieval and Rendering ===")

    backend = get_backend_service()

    # Start backend
    success = backend.start_backend()
    assert success, "Backend service should start"

    # Wait for first frame
    logger.info("Waiting for first frame...")
    frame_data = None
    max_attempts = 10

    for attempt in range(max_attempts):
        time.sleep(1)
        frame_data = backend.get_latest_frame()
        if frame_data:
            break
        logger.info(f"Attempt {attempt + 1}: No frame yet, waiting...")

    assert (
        frame_data is not None
    ), f"Should receive frame data within {max_attempts} seconds"
    assert len(frame_data) > 1000, "Frame should be substantial size (>1KB)"

    # Verify it's a JPEG by checking header
    assert frame_data.startswith(b"\xff\xd8"), "Frame should start with JPEG header"
    assert frame_data.endswith(b"\xff\xd9"), "Frame should end with JPEG footer"

    # Get metadata
    metadata = backend.get_frame_metadata()
    assert metadata is not None, "Should have frame metadata"
    assert "timestamp" in metadata, "Metadata should include timestamp"
    assert "exposure_ms" in metadata, "Metadata should include exposure"

    logger.info(
        "Frame captured successfully",
        frame_size=len(frame_data),
        exposure_ms=metadata.get("exposure_ms"),
    )

    # Try to render as image using PIL
    test_image_path = Path("test_frame.jpg")
    try:
        from PIL import Image
        import io

        image = Image.open(io.BytesIO(frame_data))
        logger.info(
            "Image loaded successfully",
            size=image.size,
            mode=image.mode,
            format=image.format,
        )

        # Save to test file to verify
        with open(test_image_path, "wb") as f:
            f.write(frame_data)
        logger.info(f"Test frame saved to {test_image_path}")

    except Exception as e:
        assert False, f"Failed to process frame as image: {e}"
    finally:
        # Clean up
        backend.stop_backend()
        if test_image_path.exists():
            test_image_path.unlink()

    logger.info("✅ Frame retrieval and rendering test passed")
    return True


def test_settings_updates_with_exposure_measurement():
    """Test settings updates with measurable exposure differences."""
    logger.info("=== Testing Settings Updates with Exposure Measurement ===")

    backend = get_backend_service()

    # Start backend
    success = backend.start_backend()
    assert success, "Backend service should start"

    # Test exposure settings: 500ms and 2000ms
    exposure_tests = [500.0, 2000.0]
    captured_exposures = []

    for target_exposure in exposure_tests:
        logger.info(f"Testing exposure setting: {target_exposure}ms")

        # Update settings
        settings_success = backend.update_settings(exposure_ms=target_exposure)
        assert (
            settings_success
        ), f"Should successfully update exposure to {target_exposure}ms"

        # Verify settings were applied
        current_settings = backend.get_current_settings()
        assert (
            current_settings["exposure_ms"] == target_exposure
        ), f"Settings should reflect {target_exposure}ms exposure"

        # Wait for settings to take effect and capture new frame
        logger.info("Waiting for settings to take effect...")
        time.sleep(3)  # Allow time for settings to propagate

        # Clear any old frames and wait for new one with updated settings
        old_frame = backend.get_latest_frame()  # Clear buffer
        time.sleep(max(target_exposure / 1000 + 1, 2))  # Wait for exposure + buffer

        # Get new frame with timing
        start_time = time.time()
        frame_data = None
        metadata = None
        max_wait = 15  # seconds

        while time.time() - start_time < max_wait:
            frame_data = backend.get_latest_frame()
            metadata = backend.get_frame_metadata()

            if frame_data and metadata:
                captured_exposure = metadata.get("exposure_ms")
                if captured_exposure == target_exposure:
                    break
            time.sleep(0.5)

        assert (
            frame_data is not None
        ), f"Should capture frame with {target_exposure}ms exposure"
        assert (
            metadata is not None
        ), f"Should have metadata for {target_exposure}ms frame"

        captured_exposure = metadata["exposure_ms"]
        captured_exposures.append(captured_exposure)

        logger.info(
            f"Captured frame with {captured_exposure}ms exposure",
            frame_size=len(frame_data),
            target_exposure=target_exposure,
        )

        # Verify exposure matches what we set (allowing small tolerance)
        tolerance = 1.0  # 1ms tolerance
        assert (
            abs(captured_exposure - target_exposure) <= tolerance
        ), f"Captured exposure {captured_exposure}ms should match target {target_exposure}ms"

    # Verify we captured different exposures
    assert len(set(captured_exposures)) == len(
        exposure_tests
    ), "Should have captured frames with different exposures"

    logger.info(
        "All exposure tests completed",
        exposures_tested=exposure_tests,
        exposures_captured=captured_exposures,
    )

    # Clean up
    backend.stop_backend()

    logger.info("✅ Settings updates with exposure measurement test passed")
    return True


def test_full_backend_workflow():
    """Integration test for complete backend workflow."""
    logger.info("=== Testing Full Backend Workflow ===")

    backend = get_backend_service()

    # Complete workflow test
    logger.info("1. Starting backend service...")
    success = backend.start_backend()
    assert success, "Backend should start"

    logger.info("2. Checking initial status...")
    status = backend.get_status()
    assert status["backend_service"] == "running"

    logger.info("3. Updating settings...")
    settings_success = backend.update_settings(exposure_ms=1000.0, gain=400)
    assert settings_success, "Settings update should succeed"

    logger.info("4. Waiting for frame capture...")
    time.sleep(3)

    logger.info("5. Retrieving frame...")
    frame_data = backend.get_latest_frame()
    assert frame_data is not None, "Should get frame data"

    logger.info("6. Getting metadata...")
    metadata = backend.get_frame_metadata()
    assert metadata is not None, "Should get metadata"
    assert metadata["exposure_ms"] == 1000.0, "Metadata should reflect updated exposure"

    logger.info("7. Performing graceful shutdown...")
    backend.shutdown_gracefully()
    assert not backend.is_started(), "Backend should be stopped after graceful shutdown"

    logger.info("✅ Full backend workflow test passed")
    return True


def main():
    """Run all tests."""
    logger.info("Starting backend service tests...")

    tests = [
        test_capture_loop_start_stop,
        test_frame_retrieval_and_rendering,
        test_settings_updates_with_exposure_measurement,
        test_full_backend_workflow,
    ]

    passed = 0
    failed = 0

    for test_func in tests:
        try:
            logger.info(f"\n{'='*50}")
            logger.info(f"Running: {test_func.__name__}")
            logger.info(f"{'='*50}")

            result = test_func()
            if result:
                passed += 1
                logger.info(f"✅ {test_func.__name__} PASSED")
            else:
                failed += 1
                logger.error(f"❌ {test_func.__name__} FAILED")

        except Exception as e:
            failed += 1
            logger.error(f"❌ {test_func.__name__} FAILED with exception", error=str(e))

    logger.info(f"\n{'='*50}")
    logger.info(f"Test Results: {passed} passed, {failed} failed")
    logger.info(f"{'='*50}")

    if failed > 0:
        logger.error("Some tests failed!")
        return 1
    else:
        logger.info("All tests passed! 🎉")
        return 0


if __name__ == "__main__":
    sys.exit(main())
