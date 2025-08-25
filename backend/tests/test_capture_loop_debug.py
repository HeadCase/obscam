#!/usr/bin/env python3
"""Debug capture loop behavior."""

import os
import sys
import time

os.environ["OBSCAM_CAMERA_TYPE"] = "gphoto2"
sys.path.insert(0, "src")

from obscam.camera_factory import get_backend_service


def debug_capture_loop():
    """Debug why capture loop stops immediately."""
    print("🔍 Debugging capture loop behavior...")

    backend = get_backend_service()

    # Start backend
    print("1. Starting backend...")
    backend.start_backend()

    # Check immediate state
    print(f"2. Backend started: {backend.is_started()}")
    print(f"3. Capture loop running: {backend.capture_loop.is_running()}")

    # Wait a bit and check again
    time.sleep(1.0)
    print(f"4. After 1s - Capture loop running: {backend.capture_loop.is_running()}")

    # Check queues
    control_queue_size = backend.capture_loop._control_queue.qsize()
    settings_queue_size = backend.capture_loop._settings_queue.qsize()
    print(f"5. Control queue size: {control_queue_size}")
    print(f"6. Settings queue size: {settings_queue_size}")

    # Wait longer
    time.sleep(3.0)
    print(
        f"7. After 4s total - Capture loop running: {backend.capture_loop.is_running()}"
    )

    # Try updating settings while running
    if backend.capture_loop.is_running():
        print("8. Updating settings...")
        backend.update_settings(exposure_ms=500.0)

        time.sleep(2.0)
        print(
            f"9. After settings update - Capture loop running: {backend.capture_loop.is_running()}"
        )

        # Check if frame has new settings
        metadata = backend.frame_buffer.get_frame_metadata()
        if metadata:
            print(f"10. Current frame exposure: {metadata.get('exposure_ms')}ms")
        else:
            print("10. No frame metadata available")
    else:
        print("8. Capture loop not running - cannot test settings update")

    backend.stop_backend()
    print("✅ Debug complete")


if __name__ == "__main__":
    debug_capture_loop()
