#!/usr/bin/env python3
"""Test script for backend service architecture with mock camera."""

import sys
import time
from pathlib import Path
import io

sys.path.insert(0, "src")

from obscam.logging_config import get_logger
from obscam.backend_service import CameraBackendService

logger = get_logger("test_backend_mock")


class MockCamera:
    """Mock camera for testing backend service without hardware."""

    def __init__(self):
        self.connected = False
        self.settings = {
            "exposure_ms": 200.0,
            "gain": 400,
            "wb_r": 100,
            "wb_b": 100,
        }
        self.frame_count = 0

    def connect(self) -> bool:
        self.connected = True
        logger.info("Mock camera connected")
        return True

    def disconnect(self) -> None:
        self.connected = False
        logger.info("Mock camera disconnected")

    def get_status(self) -> dict:
        return {
            "status": "connected" if self.connected else "disconnected",
            "camera_model": "Mock Camera",
            "is_color_camera": True,
            "current_exposure_ms": self.settings["exposure_ms"],
            "current_gain": self.settings["gain"],
            "current_wb_r": self.settings["wb_r"],
            "current_wb_b": self.settings["wb_b"],
        }

    def capture_frame(self) -> bytes | None:
        if not self.connected:
            return None

        self.frame_count += 1

        # Simulate capture time based on exposure
        exposure_seconds = self.settings["exposure_ms"] / 1000.0
        time.sleep(min(exposure_seconds, 0.1))  # Cap at 100ms for testing

        # Generate a minimal valid JPEG
        # This is a 1x1 pixel JPEG image
        minimal_jpeg = bytes(
            [
                0xFF,
                0xD8,  # JPEG header
                0xFF,
                0xE0,
                0x00,
                0x10,  # APP0 marker
                0x4A,
                0x46,
                0x49,
                0x46,  # "JFIF"
                0x00,
                0x01,
                0x01,
                0x01,  # Version
                0x00,
                0x48,
                0x00,
                0x48,  # X,Y density
                0x00,
                0x00,  # Thumbnail
                0xFF,
                0xDB,  # Quantization table marker
                0x00,
                0x43,
                0x00,  # Table length and ID
            ]
            + [0x08] * 64
            + [  # Quantization values
                0xFF,
                0xC0,  # Start of frame
                0x00,
                0x11,  # Length
                0x08,
                0x00,
                0x01,
                0x00,
                0x01,  # Precision, height, width
                0x01,
                0x01,
                0x11,
                0x00,  # Component info
                0xFF,
                0xC4,  # Huffman table
                0x00,
                0x14,
                0x00,  # Table info
            ]
            + [0x00] * 16
            + [  # Huffman lengths
                0xFF,
                0xDA,  # Start of scan
                0x00,
                0x08,  # Length
                0x01,
                0x01,
                0x00,
                0x00,
                0x3F,
                0x00,  # Scan info
                0xFF,
                0xD9,  # JPEG footer
            ]
        )

        logger.debug(
            f"Mock camera captured frame {self.frame_count}",
            frame_size=len(minimal_jpeg),
            exposure_ms=self.settings["exposure_ms"],
        )
        return minimal_jpeg

    def update_settings(self, **settings) -> bool:
        if not self.connected:
            return False

        self.settings.update(settings)
        logger.info("Mock camera settings updated", settings=settings)
        return True

    def get_current_settings(self) -> dict:
        return self.settings.copy()


def test_mock_capture_loop_start_stop():
    """Test basic capture loop start/stop functionality with mock camera."""
    logger.info("=== Testing Mock Capture Loop Start/Stop ===")

    # Create backend with mock camera
    mock_camera = MockCamera()
    cache_dir = Path("/tmp/obscam_test")
    backend = CameraBackendService(mock_camera, cache_dir)

    # Verify initial state
    assert not backend.is_started(), "Backend should not be started initially"

    # Start backend service
    logger.info("Starting backend service...")
    success = backend.start_backend()
    assert success, "Backend service should start successfully"
    assert backend.is_started(), "Backend should be marked as started"

    # Check status
    status = backend.get_status()
    logger.info("Backend status after start", status=status)
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

    logger.info("✅ Mock capture loop start/stop test passed")
    return True


def test_mock_frame_retrieval_and_rendering():
    """Test frame retrieval and verify it's a valid JPEG image with mock camera."""
    logger.info("=== Testing Mock Frame Retrieval and Rendering ===")

    # Create backend with mock camera
    mock_camera = MockCamera()
    cache_dir = Path("/tmp/obscam_test")
    backend = CameraBackendService(mock_camera, cache_dir)

    # Start backend
    success = backend.start_backend()
    assert success, "Backend service should start"

    # Wait for first frame
    logger.info("Waiting for first frame...")
    frame_data = None
    max_attempts = 5

    for attempt in range(max_attempts):
        time.sleep(1)
        frame_data = backend.get_latest_frame()
        if frame_data:
            break
        logger.info(f"Attempt {attempt + 1}: No frame yet, waiting...")

    assert (
        frame_data is not None
    ), f"Should receive frame data within {max_attempts} seconds"
    assert len(frame_data) > 100, "Frame should be substantial size (>100 bytes)"

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
    test_image_path = Path("test_mock_frame.jpg")
    try:
        from PIL import Image

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

    logger.info("✅ Mock frame retrieval and rendering test passed")
    return True


def test_mock_settings_updates_with_exposure_measurement():
    """Test settings updates with measurable exposure differences using mock camera."""
    logger.info("=== Testing Mock Settings Updates with Exposure Measurement ===")

    # Create backend with mock camera
    mock_camera = MockCamera()
    cache_dir = Path("/tmp/obscam_test")
    backend = CameraBackendService(mock_camera, cache_dir)

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
        time.sleep(2)  # Allow time for settings to propagate

        # Get new frame
        frame_data = backend.get_latest_frame()
        metadata = backend.get_frame_metadata()

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

        # Verify exposure matches what we set
        assert (
            captured_exposure == target_exposure
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

    logger.info("✅ Mock settings updates with exposure measurement test passed")
    return True


def test_mock_full_backend_workflow():
    """Integration test for complete backend workflow with mock camera."""
    logger.info("=== Testing Mock Full Backend Workflow ===")

    # Create backend with mock camera
    mock_camera = MockCamera()
    cache_dir = Path("/tmp/obscam_test")
    backend = CameraBackendService(mock_camera, cache_dir)

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
    time.sleep(2)

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

    logger.info("✅ Mock full backend workflow test passed")
    return True


def main():
    """Run all mock tests."""
    logger.info("Starting mock backend service tests...")

    tests = [
        test_mock_capture_loop_start_stop,
        test_mock_frame_retrieval_and_rendering,
        test_mock_settings_updates_with_exposure_measurement,
        test_mock_full_backend_workflow,
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
        logger.info("All mock tests passed! 🎉")
        logger.info("Architecture validation complete - ready for real camera testing")
        return 0


if __name__ == "__main__":
    sys.exit(main())
