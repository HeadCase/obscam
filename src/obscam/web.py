import threading
import time
from pathlib import Path

import uvicorn
from fastapi import FastAPI, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse, JSONResponse
from flask import Flask, abort, render_template, send_file

from .camera import get_camera, get_storage_manager, ProgramMode

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

# Initialize camera and storage manager
camera = get_camera()
storage_manager = get_storage_manager()


@flask_app.route("/")
def index():
    """Main page displaying the camera image."""
    return render_template("index.html")


@flask_app.route("/images/<filename>")
def serve_image(filename):
    """Serve camera images."""
    # Use absolute path from project root
    image_path = Path(__file__).parent.parent.parent / "images" / filename
    if image_path.exists():
        return send_file(image_path)
    abort(404)


@fastapi_app.get("/api/status")
async def get_status():
    """Get current camera status with storage information."""
    try:
        camera_status = camera.get_camera_status()
        
        # Add storage information
        storage_stats = storage_manager.get_storage_stats()
        storage_info = {
            "storage_used_gb": storage_stats.get("total_size_gb", 0),
            "storage_used_percent": storage_stats.get("storage_used_percent", 0),
            "storage_max_gb": storage_stats.get("max_storage_gb", 3),
            "total_images": storage_stats.get("total_images", 0),
            "days_until_full": storage_stats.get("days_until_full", -1)
        }
        
        return JSONResponse(
            {
                **camera_status,
                **storage_info,
                "timestamp": time.time(),
            }
        )
    except Exception as e:
        return JSONResponse(
            {
                "status": "error",
                "message": f"Failed to get camera status: {e}",
                "timestamp": time.time(),
            }
        )


@fastapi_app.get("/api/connect")
async def connect_camera():
    """Connect to the camera."""
    try:
        if camera.connect():
            return JSONResponse(
                {
                    "status": "connected",
                    "message": "Camera connected successfully",
                    "timestamp": time.time(),
                }
            )
        else:
            return JSONResponse(
                {
                    "status": "error",
                    "message": "Failed to connect to camera",
                    "timestamp": time.time(),
                }
            )
    except Exception as e:
        return JSONResponse(
            {
                "status": "error",
                "message": f"Connection error: {e}",
                "timestamp": time.time(),
            }
        )


@fastapi_app.get("/api/capture")
async def capture_image():
    """Capture a new image from the camera with auto mode detection."""
    try:
        if not camera.is_initialized:
            raise HTTPException(status_code=400, detail="Camera not connected")

        image_path = camera.capture_image()
        if image_path:
            filename = Path(image_path).name
            status = camera.get_camera_status()
            return JSONResponse(
                {
                    "status": "success",
                    "image_path": image_path,
                    "image_url": f"/images/{filename}",
                    "filename": filename,
                    "timestamp": time.time(),
                    "message": "Image captured successfully",
                    "capture_mode": status.get("capture_mode", "unknown"),
                    "scene_type": status.get("scene_type", "unknown"),
                    "refresh_interval": status.get("refresh_interval", 1000),
                }
            )
        else:
            raise HTTPException(status_code=500, detail="Failed to capture image")

    except Exception as e:
        return JSONResponse(
            {
                "status": "error",
                "message": f"Capture failed: {e}",
                "timestamp": time.time(),
            }
        )


@fastapi_app.get("/api/capture/auto")
async def capture_auto():
    """AUTO mode: Adaptive exposure with low gain, handles lighting changes."""
    try:
        if not camera.is_initialized:
            raise HTTPException(status_code=400, detail="Camera not connected")

        # Set to AUTO mode if not already
        camera.set_program_mode(ProgramMode.AUTO)
        
        image_path = camera.capture_image()
        if image_path:
            filename = Path(image_path).name
            return JSONResponse(
                {
                    "status": "success",
                    "image_path": image_path,
                    "image_url": f"/images/{filename}",
                    "filename": filename,
                    "timestamp": time.time(),
                    "message": "AUTO capture completed",
                    "program_mode": "auto",
                    "refresh_interval": 500,
                }
            )
        else:
            raise HTTPException(status_code=500, detail="AUTO capture failed")

    except Exception as e:
        return JSONResponse(
            {
                "status": "error",
                "message": f"AUTO capture failed: {e}",
                "timestamp": time.time(),
            }
        )


@fastapi_app.get("/api/capture/slewing")
async def capture_slewing():
    """SLEWING mode: Max 500ms exposure for real-time mount monitoring."""
    try:
        if not camera.is_initialized:
            raise HTTPException(status_code=400, detail="Camera not connected")

        # Set to SLEWING mode if not already
        camera.set_program_mode(ProgramMode.SLEWING)
        
        image_path = camera.capture_image()
        if image_path:
            filename = Path(image_path).name
            return JSONResponse(
                {
                    "status": "success",
                    "image_path": image_path,
                    "image_url": f"/images/{filename}",
                    "filename": filename,
                    "timestamp": time.time(),
                    "message": "SLEWING capture completed",
                    "program_mode": "slewing", 
                    "refresh_interval": 250,  # Faster for slewing
                }
            )
        else:
            raise HTTPException(status_code=500, detail="SLEWING capture failed")

    except Exception as e:
        return JSONResponse(
            {
                "status": "error",
                "message": f"SLEWING capture failed: {e}",
                "timestamp": time.time(),
            }
        )


@fastapi_app.get("/api/capture/manual/{exposure_ms}/{gain}")
async def capture_manual(exposure_ms: int, gain: int):
    """MANUAL capture with specific exposure (ms) and gain values."""
    try:
        if not camera.is_initialized:
            raise HTTPException(status_code=400, detail="Camera not connected")

        # Convert ms to microseconds
        exposure_us = exposure_ms * 1000
        
        # Set to MANUAL mode with specific settings
        camera.set_program_mode(ProgramMode.MANUAL, exposure_us=exposure_us, gain=gain)
        
        image_path = camera.capture_image()
        if image_path:
            filename = Path(image_path).name
            return JSONResponse(
                {
                    "status": "success", 
                    "image_path": image_path,
                    "image_url": f"/images/{filename}",
                    "filename": filename,
                    "timestamp": time.time(),
                    "message": f"MANUAL capture completed: {exposure_ms}ms, gain {gain}",
                    "program_mode": "manual",
                    "manual_exposure_ms": exposure_ms,
                    "manual_gain": gain,
                    "refresh_interval": max(500, exposure_ms + 100),  # Based on exposure time
                }
            )
        else:
            raise HTTPException(status_code=500, detail="MANUAL capture failed")

    except Exception as e:
        return JSONResponse(
            {
                "status": "error",
                "message": f"MANUAL capture failed: {e}",
                "timestamp": time.time(),
            }
        )


@fastapi_app.get("/api/image")
async def get_current_image():
    """Get the latest captured image."""
    try:
        image_dir = Path("images")
        if not image_dir.exists():
            raise HTTPException(status_code=404, detail="No images directory")

        # Find the most recent image
        image_files = list(image_dir.glob("*.jpg")) + list(image_dir.glob("*.jpeg"))
        if not image_files:
            return JSONResponse(
                {
                    "status": "no_images",
                    "message": "No images available",
                    "timestamp": time.time(),
                }
            )

        latest_image = max(image_files, key=lambda p: p.stat().st_mtime)
        filename = latest_image.name

        return JSONResponse(
            {
                "status": "success",
                "image_url": f"/images/{filename}",
                "filename": filename,
                "image_path": str(latest_image),
                "timestamp": time.time(),
                "message": "Latest image retrieved",
            }
        )

    except Exception as e:
        return JSONResponse(
            {
                "status": "error",
                "message": f"Failed to get image: {e}",
                "timestamp": time.time(),
            }
        )


@fastapi_app.get("/images/{filename}")
async def serve_image_api(filename: str):
    """Serve camera images via API."""
    image_path = Path("images") / filename
    if image_path.exists():
        return FileResponse(image_path)
    raise HTTPException(status_code=404, detail="Image not found")


@fastapi_app.get("/api/settings")
async def get_camera_settings():
    """Get current camera settings."""
    try:
        if not camera.is_initialized:
            raise HTTPException(status_code=400, detail="Camera not connected")
        
        status = camera.get_camera_status()
        return JSONResponse({
            "status": "success",
            "settings": status.get("current_settings", {}),
            "timestamp": time.time()
        })
    except Exception as e:
        return JSONResponse({
            "status": "error", 
            "message": f"Failed to get settings: {e}",
            "timestamp": time.time()
        })


@fastapi_app.post("/api/settings")
async def update_camera_settings(settings: dict):
    """Update camera settings."""
    try:
        if not camera.is_initialized:
            raise HTTPException(status_code=400, detail="Camera not connected")
        
        # Update exposure if provided
        if "exposure" in settings:
            import zwoasi as asi
            camera.camera.set_control_value(asi.ASI_EXPOSURE, int(settings["exposure"]))
        
        # Update gain if provided  
        if "gain" in settings:
            import zwoasi as asi
            camera.camera.set_control_value(asi.ASI_GAIN, int(settings["gain"]))
            
        return JSONResponse({
            "status": "success",
            "message": "Settings updated",
            "timestamp": time.time()
        })
    except Exception as e:
        return JSONResponse({
            "status": "error",
            "message": f"Failed to update settings: {e}", 
            "timestamp": time.time()
        })


# Storage Management APIs
@fastapi_app.get("/api/storage/stats")
async def get_storage_stats():
    """Get storage statistics and usage information."""
    try:
        stats = storage_manager.get_storage_stats()
        
        # Add formatted display values
        if "error" not in stats:
            stats["storage_display"] = f"{stats['storage_used_percent']}% used ({stats['total_size_gb']}GB / {stats['max_storage_gb']}GB)"
            
            # Format oldest/newest dates
            if stats["oldest_image"]:
                stats["oldest_image_date"] = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(stats["oldest_image"]))
            if stats["newest_image"]:
                stats["newest_image_date"] = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(stats["newest_image"]))
        
        return JSONResponse({
            "status": "success" if "error" not in stats else "error",
            "data": stats,
            "timestamp": time.time()
        })
    except Exception as e:
        return JSONResponse({
            "status": "error",
            "message": f"Failed to get storage stats: {e}",
            "timestamp": time.time()
        })


@fastapi_app.post("/api/storage/cleanup")
async def cleanup_storage(force: bool = False):
    """Manually trigger storage cleanup."""
    try:
        result = storage_manager.cleanup_old_images(force=force)
        
        if "error" in result:
            return JSONResponse({
                "status": "error",
                "message": result["error"],
                "timestamp": time.time()
            })
        
        return JSONResponse({
            "status": "success",
            "data": result,
            "message": result.get("message", "Cleanup completed"),
            "timestamp": time.time()
        })
    except Exception as e:
        return JSONResponse({
            "status": "error", 
            "message": f"Cleanup failed: {e}",
            "timestamp": time.time()
        })


@fastapi_app.get("/api/storage/images")
async def list_images(limit: int = 50, offset: int = 0):
    """List stored images with metadata."""
    try:
        image_dir = Path("images")
        if not image_dir.exists():
            return JSONResponse({
                "status": "success",
                "data": {"images": [], "total_count": 0},
                "timestamp": time.time()
            })
        
        # Get all image files
        image_files = list(image_dir.glob("*.jpg")) + list(image_dir.glob("*.jpeg"))
        image_files.sort(key=lambda p: p.stat().st_mtime, reverse=True)  # Newest first
        
        # Apply pagination
        total_count = len(image_files)
        paginated_files = image_files[offset:offset + limit]
        
        # Build response data
        images = []
        for img_file in paginated_files:
            stat = img_file.stat()
            images.append({
                "filename": img_file.name,
                "url": f"/images/{img_file.name}",
                "size_mb": round(stat.st_size / (1024 * 1024), 2),
                "created": stat.st_mtime,
                "created_date": time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(stat.st_mtime))
            })
        
        return JSONResponse({
            "status": "success",
            "data": {
                "images": images,
                "total_count": total_count,
                "limit": limit,
                "offset": offset,
                "has_more": offset + limit < total_count
            },
            "timestamp": time.time()
        })
    except Exception as e:
        return JSONResponse({
            "status": "error",
            "message": f"Failed to list images: {e}",
            "timestamp": time.time()
        })


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
    
    # Start background cleanup service
    print("Starting storage cleanup service...")
    storage_manager.start_background_cleanup()
    
    # Start Flask in a separate thread
    flask_thread = threading.Thread(target=run_flask, daemon=True)
    flask_thread.start()

    # Start FastAPI in the main thread
    print("Starting web servers...")
    print("Flask (web pages): http://localhost:5000")
    print("FastAPI (API): http://localhost:8000")
    print(f"Storage management: 3-day retention, {storage_manager.MAX_STORAGE_GB}GB max")
    run_fastapi()


if __name__ == "__main__":
    start_web_servers()

