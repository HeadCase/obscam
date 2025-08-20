import threading
import time

import uvicorn
from fastapi import FastAPI, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import Response
from flask import Flask, render_template

from .camera import get_camera

# Flask app for serving web pages
flask_app = Flask(__name__, template_folder="templates")

# FastAPI app for API endpoints
fastapi_app = FastAPI()

# Add CORS middleware for cross-origin requests
fastapi_app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# Initialize camera
camera = get_camera()


@flask_app.route("/")
def index():
    """Main page displaying the camera feed."""
    return render_template("index.html")


@fastapi_app.get("/api/status")
async def get_status():
    """Get current camera status."""
    try:
        camera_status = camera.get_status()
        return {
            **camera_status,
            "timestamp": time.time(),
        }
    except Exception as e:
        return {
            "status": "error",
            "message": f"Failed to get camera status: {e}",
            "timestamp": time.time(),
        }


@fastapi_app.get("/api/connect")
async def connect_camera():
    """Connect to the camera."""
    try:
        if camera.connect():
            return {
                "status": "connected",
                "message": "Camera connected successfully",
                "timestamp": time.time(),
            }
        else:
            return {
                "status": "error",
                "message": "Failed to connect to camera",
                "timestamp": time.time(),
            }
    except Exception as e:
        return {
            "status": "error",
            "message": f"Connection error: {e}",
            "timestamp": time.time(),
        }


@fastapi_app.get("/api/capture-fast")
async def capture_fast(exp: int = 100, gain: int = 250):
    """Ultra-fast capture endpoint - returns raw JPEG bytes.
    
    Args:
        exp: Exposure time in milliseconds
        gain: Camera gain value
    """
    try:
        if not camera.is_initialized:
            raise HTTPException(status_code=400, detail="Camera not connected")

        # Convert milliseconds to microseconds
        exposure_us = exp * 1000
        
        # Capture directly to memory
        jpeg_bytes = camera.capture_fast(exposure_us, gain)
        
        if jpeg_bytes:
            # Return raw JPEG bytes with proper content type
            return Response(
                content=jpeg_bytes,
                media_type="image/jpeg",
                headers={
                    "Cache-Control": "no-cache, no-store, must-revalidate",
                    "Pragma": "no-cache",
                    "Expires": "0"
                }
            )
        else:
            raise HTTPException(status_code=500, detail="Capture failed")

    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Capture failed: {e}")


def run_flask():
    """Run Flask app in a separate thread."""
    flask_app.run(host="0.0.0.0", port=5000, debug=False, use_reloader=False)


def run_fastapi():
    """Run FastAPI app in a separate thread."""
    uvicorn.run(fastapi_app, host="0.0.0.0", port=8000, log_level="info")


def start_web_servers():
    """Start both Flask and FastAPI servers."""
    # Connect to camera on startup
    print("Initializing camera...")
    if camera.connect():
        print("Camera connected successfully!")
    else:
        print("Warning: Failed to connect to camera. Use /api/connect to retry.")
    
    # Start Flask in a separate thread
    flask_thread = threading.Thread(target=run_flask, daemon=True)
    flask_thread.start()

    # Start FastAPI in the main thread
    print("Starting web servers...")
    print("Flask (web pages): http://localhost:5000")
    print("FastAPI (API): http://localhost:8000")
    print("Fast capture endpoint: http://localhost:8000/api/capture-fast?exp=100&gain=250")
    run_fastapi()


if __name__ == "__main__":
    start_web_servers()