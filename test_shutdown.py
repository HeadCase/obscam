#!/usr/bin/env python3
"""Test script to verify graceful shutdown of obscam."""

import subprocess
import time
import signal
import sys


def test_graceful_shutdown():
    print("Starting obscam process...")

    # Start the obscam process
    process = subprocess.Popen(
        ["uv", "run", "obscam"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        cwd="/home/gheadley/dev/obscam",
    )

    print(f"Process started with PID: {process.pid}")
    print("Waiting 3 seconds for startup...")
    time.sleep(3)

    print("\nSending SIGINT (Ctrl+C) to the process...")
    process.send_signal(signal.SIGINT)

    print("Waiting for graceful shutdown...")

    # Collect output while waiting for process to terminate
    try:
        output, _ = process.communicate(timeout=5)
        print("\nProcess output:")
        print("-" * 50)
        # Print last 30 lines to see shutdown messages
        lines = output.split("\n")
        for line in lines[-30:]:
            if line:
                print(line)
        print("-" * 50)

        if process.returncode == 0:
            print(f"\n✓ Process exited cleanly with code: {process.returncode}")
        else:
            print(f"\n✗ Process exited with code: {process.returncode}")
    except subprocess.TimeoutExpired:
        print("\n✗ Process did not shut down within 5 seconds, forcing termination...")
        process.kill()
        output, _ = process.communicate()
        print("Process forcefully terminated")
        return False

    # Check for graceful shutdown messages in output
    if "graceful shutdown" in output.lower() or "cleanup completed" in output.lower():
        print("\n✓ Found graceful shutdown messages in output")
        return True
    else:
        print("\n⚠ Did not find expected graceful shutdown messages")
        return False


if __name__ == "__main__":
    success = test_graceful_shutdown()
    sys.exit(0 if success else 1)
