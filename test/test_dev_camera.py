#!/usr/bin/env python3
"""Test script for development camera setup."""

import os
import sys
import time

# Set environment for gphoto2 camera
os.environ["OBSCAM_CAMERA_TYPE"] = "gphoto2"

sys.path.insert(0, "src")

from obscam.camera_factory import get_camera


def main():
    print("Testing camera abstraction layer...")
    print(f"OBSCAM_CAMERA_TYPE: {os.getenv('OBSCAM_CAMERA_TYPE')}")

    camera = get_camera()
    print(f"Camera type: {camera.__class__.__name__}")

    if not camera.connect():
        print("Failed to connect to camera")
        return 1

    print("Camera connected successfully")
    status = camera.get_status()
    print(f"Status: {status}")

    # Start continuous capture
    print("\nStarting continuous capture...")
    if not camera.start_continuous_capture():
        print("Failed to start continuous capture")
        camera.disconnect()
        return 1

    print("Waiting for frames...")
    for i in range(3):
        time.sleep(1)
        frame = camera.get_latest_frame()
        metadata = camera.get_frame_metadata()

        if frame:
            print(f"Frame {i+1}: {len(frame)} bytes")
            if metadata:
                print(
                    f"  Settings: exposure={metadata['exposure_ms']}ms, gain={metadata['gain']}"
                )
        else:
            print(f"Frame {i+1}: No frame available yet")

    print("\nStopping continuous capture...")
    camera.stop_continuous_capture()

    print("Disconnecting...")
    camera.disconnect()

    print("\nTest completed successfully!")
    return 0


if __name__ == "__main__":
    sys.exit(main())
