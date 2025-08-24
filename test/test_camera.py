#!/usr/bin/env python3
"""
Simple test script for the ASI662MC camera integration.
This script tests basic camera functionality without starting the web server.
"""

import sys
import os
sys.path.insert(0, 'src')

from obscam.camera import ASI662MCCamera

def test_camera():
    """Test basic camera functionality."""
    print("=== ObsCam ASI662MC Test ===")
    
    # Create camera instance
    print("1. Creating camera instance...")
    camera = ASI662MCCamera()
    
    # Try to connect
    print("2. Attempting to connect to camera...")
    if not camera.connect():
        print("   ERROR: Failed to connect to camera")
        print("   Make sure:")
        print("   - ASI662MC camera is connected via USB")
        print("   - ZWO ASI SDK library is installed")
        print("   - ZWO_ASI_LIB environment variable is set (optional)")
        return False
    
    print("   SUCCESS: Camera connected!")
    
    # Get camera status
    print("3. Getting camera status...")
    status = camera.get_camera_status()
    print(f"   Status: {status}")
    
    # Try to capture an image
    print("4. Capturing test image...")
    image_path = camera.capture_image("test_image.jpg")
    if image_path:
        print(f"   SUCCESS: Image saved to {image_path}")
    else:
        print("   ERROR: Failed to capture image")
        return False
    
    # Disconnect camera
    print("5. Disconnecting camera...")
    camera.disconnect()
    print("   Camera disconnected")
    
    print("\n=== Test completed successfully! ===")
    print("You can now run the web interface with: uv run obscam")
    return True

if __name__ == "__main__":
    try:
        success = test_camera()
        sys.exit(0 if success else 1)
    except KeyboardInterrupt:
        print("\nTest interrupted by user")
        sys.exit(1)
    except Exception as e:
        print(f"\nTest failed with error: {e}")
        sys.exit(1)