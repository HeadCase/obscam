#!/usr/bin/env python3
"""
Setup script to help configure the ZWO ASI SDK for ObsCam.
This script helps locate and configure the ZWO ASI SDK library.
"""

import os
import subprocess
from pathlib import Path

def find_asi_library():
    """Try to find the ZWO ASI SDK library on the system."""
    print("Searching for ZWO ASI SDK library...")
    
    # Common library names and paths
    library_names = [
        'libASICamera2.so',
        'libASICamera2.so.1', 
        'ASICamera2.dll'  # Windows
    ]
    
    search_paths = [
        '/usr/local/lib',
        '/usr/lib', 
        '/usr/lib/x86_64-linux-gnu',
        '/usr/lib/arm-linux-gnueabihf',  # Raspberry Pi
        '/usr/lib/aarch64-linux-gnu',    # Raspberry Pi 64-bit
        '/opt/ASI',
        '.',
        './lib'
    ]
    
    found_libraries = []
    
    for path in search_paths:
        path_obj = Path(path)
        if path_obj.exists():
            for lib_name in library_names:
                lib_path = path_obj / lib_name
                if lib_path.exists():
                    found_libraries.append(str(lib_path))
                    
    return found_libraries

def check_zwo_asi_lib_env():
    """Check if ZWO_ASI_LIB environment variable is set."""
    env_lib = os.getenv('ZWO_ASI_LIB')
    if env_lib:
        if Path(env_lib).exists():
            print(f"✓ ZWO_ASI_LIB environment variable set to: {env_lib}")
            return env_lib
        else:
            print(f"✗ ZWO_ASI_LIB set to {env_lib}, but file does not exist")
    else:
        print("• ZWO_ASI_LIB environment variable not set")
    return None

def check_camera_connection():
    """Check if ASI camera is connected via USB."""
    print("\nChecking for connected ASI cameras...")
    
    try:
        # Use lsusb to check for ZWO cameras
        result = subprocess.run(['lsusb'], capture_output=True, text=True)
        if result.returncode == 0:
            lines = result.stdout.split('\n')
            zwo_devices = [line for line in lines if 'ZWO' in line or '03c3:' in line]
            if zwo_devices:
                print("✓ Found ZWO camera(s):")
                for device in zwo_devices:
                    print(f"  {device.strip()}")
                return True
            else:
                print("✗ No ZWO cameras found via USB")
        else:
            print("• Could not check USB devices (lsusb not available)")
    except FileNotFoundError:
        print("• lsusb command not found, cannot check USB devices")
    
    return False

def main():
    """Main setup function."""
    print("=== ObsCam ZWO ASI SDK Setup ===\n")
    
    # Check environment variable
    env_lib = check_zwo_asi_lib_env()
    
    # Search for libraries
    found_libs = find_asi_library()
    
    if found_libs:
        print(f"\n✓ Found {len(found_libs)} ASI SDK libraries:")
        for lib in found_libs:
            print(f"  {lib}")
            
        if not env_lib:
            recommended_lib = found_libs[0]
            print(f"\n💡 Recommended action:")
            print(f"   Set ZWO_ASI_LIB environment variable:")
            print(f"   export ZWO_ASI_LIB={recommended_lib}")
            print(f"   \n   Or add to your ~/.bashrc:")
            print(f"   echo 'export ZWO_ASI_LIB={recommended_lib}' >> ~/.bashrc")
    else:
        print("\n✗ No ASI SDK libraries found")
        print("   You need to install the ZWO ASI SDK:")
        print("   1. Download from: https://astronomy-imaging-camera.com/software-drivers")
        print("   2. Extract and copy libASICamera2.so to /usr/local/lib")
        print("   3. Run: sudo ldconfig")
    
    # Check camera connection
    check_camera_connection()
    
    print(f"\n=== Next Steps ===")
    print("1. Connect your ASI662MC camera via USB")
    print("2. Set the ZWO_ASI_LIB environment variable (if not already set)")
    print("3. Run the camera test: python test_camera.py")
    print("4. Start the web interface: uv run obscam")

if __name__ == "__main__":
    main()