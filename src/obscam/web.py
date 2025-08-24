import threading
import time

import uvicorn
from fastapi import FastAPI, HTTPException, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import Response
from flask import Flask, render_template

from .camera_factory import get_camera


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


@fastapi_app.get("/api/latest-frame")
async def get_latest_frame():
    """Get the latest cached frame (stateless, no session required)."""
    try:
        status = camera.get_status()
        if status.get("status") != "connected":
            raise HTTPException(status_code=400, detail="Camera not connected")

        frame_bytes = camera.get_latest_frame()
        frame_metadata = camera.get_frame_metadata()

        if frame_bytes:
            # Calculate frame age for honest timestamp reporting
            current_time = time.time()
            frame_timestamp = (
                frame_metadata.get("timestamp", 0) if frame_metadata else 0
            )
            frame_age_seconds = (
                current_time - frame_timestamp if frame_timestamp > 0 else 0
            )

            return Response(
                content=frame_bytes,
                media_type="image/jpeg",
                headers={
                    "Cache-Control": "no-cache, no-store, must-revalidate",
                    "Pragma": "no-cache",
                    "Expires": "0",
                    "X-Frame-Timestamp": str(frame_timestamp),
                    "X-Frame-Age-Seconds": str(round(frame_age_seconds, 2)),
                },
            )
        else:
            raise HTTPException(status_code=503, detail="No frame available")

    except HTTPException:
        raise
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Frame retrieval failed: {e}")


@fastapi_app.post("/api/update-settings")
async def update_settings(request: Request):
    """Update camera settings (stateless, no session required)."""
    try:
        status = camera.get_status()
        if status.get("status") != "connected":
            raise HTTPException(status_code=400, detail="Camera not connected")

        data = await request.json()

        # Validate and sanitize settings
        valid_settings = {}
        if "exposure_ms" in data:
            exposure_ms = float(data["exposure_ms"])
            if 0.1 <= exposure_ms <= 30000:  # 0.1ms to 30s range
                valid_settings["exposure_ms"] = exposure_ms
            else:
                raise HTTPException(
                    status_code=400, detail="Exposure must be between 0.1ms and 30s"
                )

        if "gain" in data:
            gain = int(data["gain"])
            if 0 <= gain <= 1000:  # Typical ZWO gain range
                valid_settings["gain"] = gain
            else:
                raise HTTPException(
                    status_code=400, detail="Gain must be between 0 and 1000"
                )

        if "wb_r" in data:
            wb_r = int(data["wb_r"])
            if 50 <= wb_r <= 150:  # White balance range
                valid_settings["wb_r"] = wb_r
            else:
                raise HTTPException(
                    status_code=400,
                    detail="Red white balance must be between 50 and 150",
                )

        if "wb_b" in data:
            wb_b = int(data["wb_b"])
            if 50 <= wb_b <= 150:  # White balance range
                valid_settings["wb_b"] = wb_b
            else:
                raise HTTPException(
                    status_code=400,
                    detail="Blue white balance must be between 50 and 150",
                )

        if not valid_settings:
            raise HTTPException(status_code=400, detail="No valid settings provided")

        success = camera.update_settings(**valid_settings)

        if success:
            current_settings = camera.get_current_settings()
            return {
                "status": "success",
                "message": "Settings updated",
                "current_settings": current_settings,
                "timestamp": time.time(),
            }
        else:
            raise HTTPException(
                status_code=500, detail="Failed to update camera settings"
            )

    except HTTPException:
        raise
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Settings update failed: {e}")


@fastapi_app.get("/api/frame-info")
async def get_frame_info():
    """Get information about the latest frame and continuous capture status."""
    try:
        status = camera.get_status()
        if status.get("status") != "connected":
            raise HTTPException(status_code=400, detail="Camera not connected")

        frame_metadata = camera.get_frame_metadata()
        current_settings = camera.get_current_settings()

        return {
            "has_frame": frame_metadata is not None,
            "frame_metadata": frame_metadata,
            "current_settings": current_settings,
            "timestamp": time.time(),
        }

    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Frame info failed: {e}")


@fastapi_app.post("/api/start-background")
async def start_continuous_capture():
    """Start continuous capture thread."""
    try:
        status = camera.get_status()
        if status.get("status") != "connected":
            raise HTTPException(status_code=400, detail="Camera not connected")

        success = camera.start_continuous_capture()

        if success:
            return {
                "status": "success",
                "message": "Continuous capture started",
                "timestamp": time.time(),
            }
        else:
            raise HTTPException(
                status_code=500, detail="Failed to start continuous capture"
            )

    except Exception as e:
        raise HTTPException(
            status_code=500, detail=f"Continuous capture start failed: {e}"
        )


@fastapi_app.post("/api/stop-background")
async def stop_continuous_capture():
    """Stop continuous capture thread."""
    try:
        camera.stop_continuous_capture()

        return {
            "status": "success",
            "message": "Continuous capture stopped",
            "timestamp": time.time(),
        }

    except Exception as e:
        raise HTTPException(
            status_code=500, detail=f"Continuous capture stop failed: {e}"
        )


def run_flask(port: int = 5000):
    """Run Flask app in a separate thread."""
    flask_app.run(host="0.0.0.0", port=port, debug=False, use_reloader=False)


def run_fastapi():
    """Run FastAPI app in a separate thread."""
    uvicorn.run(fastapi_app, host="0.0.0.0", port=8000, log_level="info")


def start_web_servers(flask_port=5000, fastapi_port=8000):
    """Start both Flask and FastAPI servers."""
    # Connect to camera on startup
    print("Initializing camera...")
    if camera.connect():
        print("Camera connected successfully!")

        # Start continuous capture for stateless frame delivery
        print("Starting continuous capture...")
        if camera.start_continuous_capture():
            print("Continuous capture started - stateless frame access now available!")
            print("New endpoints:")
            print("  - Stateless frame: http://localhost:8000/api/latest-frame")
            print("  - Update settings: POST http://localhost:8000/api/update-settings")
            print("  - Frame info: http://localhost:8000/api/frame-info")
        else:
            print("Warning: Continuous capture failed to start")
    else:
        print("Warning: Failed to connect to camera. Use /api/connect to retry.")

    # Start Flask in a separate thread
    flask_thread = threading.Thread(
        target=run_flask, kwargs={"port": flask_port}, daemon=True
    )
    flask_thread.start()

    # Start FastAPI in the main thread
    print("Starting web servers...")
    print("Flask (web pages): http://localhost:5000")
    print("FastAPI (API): http://localhost:8000")
    print(
        "Fast capture endpoint: http://localhost:8000/api/capture-fast?exp=100&gain=250"
    )

    run_fastapi()


if __name__ == "__main__":
    start_web_servers()
