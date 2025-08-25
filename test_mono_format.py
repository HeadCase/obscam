#!/usr/bin/env python3
"""Test script to verify monochrome image format implementation."""

import sys

sys.path.insert(0, "/home/gheadley/develop/obscam/backend/src")

from obscam.camera.zwo_asi_camera import ZwoAsiCamera
from obscam.camera.libgphoto2_camera import Gphoto2Camera


def test_zwo_camera():
    """Test ZWO camera with mono/color formats."""
    print("Testing ZWO ASI Camera...")
    camera = ZwoAsiCamera()

    # Check default settings
    settings = camera.get_current_settings()
    print(f"Default settings: {settings}")
    assert settings.get("image_format") == "mono", "ZWO should default to mono"

    # Test updating to color
    success = camera.update_settings(image_format="color")
    print(f"Update to color: {success}")

    settings = camera.get_current_settings()
    assert settings.get("image_format") == "color", "Should be color after update"

    # Test updating back to mono
    success = camera.update_settings(image_format="mono")
    print(f"Update to mono: {success}")

    settings = camera.get_current_settings()
    assert settings.get("image_format") == "mono", "Should be mono after update"

    # Check capabilities
    caps = camera.get_control_capabilities()
    print(f"Image format capability: {caps.get('image_format')}")

    print("✓ ZWO camera tests passed\n")


def test_libgphoto2_camera():
    """Test libgphoto2 camera with color format."""
    print("Testing libgphoto2 Camera...")
    camera = Gphoto2Camera()

    # Check default settings
    settings = camera.get_current_settings()
    print(f"Default settings: {settings}")
    assert settings.get("image_format") == "color", "Libgphoto2 should default to color"

    # Test that it accepts color format
    success = camera.update_settings(image_format="color")
    print(f"Update to color: {success}")

    # Test that it accepts mono (for metadata only)
    success = camera.update_settings(image_format="mono")
    print(f"Update to mono (metadata only): {success}")

    settings = camera.get_current_settings()
    assert settings.get("image_format") == "mono", "Should accept mono for metadata"

    # Check capabilities
    caps = camera.get_control_capabilities()
    print(f"Image format capability: {caps.get('image_format')}")

    print("✓ libgphoto2 camera tests passed\n")


if __name__ == "__main__":
    print("Testing monochrome image format implementation\n")
    print("=" * 50)

    try:
        test_zwo_camera()
        test_libgphoto2_camera()
        print("=" * 50)
        print("✅ All tests passed!")
    except AssertionError as e:
        print(f"❌ Test failed: {e}")
        sys.exit(1)
    except Exception as e:
        print(f"❌ Error: {e}")
        sys.exit(1)
