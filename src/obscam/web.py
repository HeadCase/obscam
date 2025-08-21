import threading
import time
import json
from urllib.parse import parse_qs

import uvicorn
from fastapi import FastAPI, HTTPException, WebSocket, WebSocketDisconnect, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import Response
from flask import Flask, render_template, request

from .camera import get_camera
from .session_manager import get_session_manager

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

# Initialize camera and session manager
camera = get_camera()
session_manager = get_session_manager()


@flask_app.route("/")
def index():
    """Main page displaying the camera feed."""
    # Check for force_master parameter
    force_master = request.args.get("force_master", "false").lower() == "true"
    return render_template("index.html", force_master=force_master)


@fastapi_app.get("/api/status")
async def get_status():
    """Get current camera status with session information."""
    try:
        camera_status = camera.get_status()
        session_info = session_manager.get_session_info()

        return {
            **camera_status,
            "session_info": session_info,
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
async def capture_fast(
    exp: int = 200,
    gain: int = 250,
    wb_r: int = None,
    wb_b: int = None,
    session_id: str = None,
):
    """Ultra-fast capture endpoint with STRICT master/observer control.

    Args:
        exp: Exposure time in milliseconds (IGNORED for observers)
        gain: Camera gain value (IGNORED for observers)
        wb_r: Red white balance 50-150 (IGNORED for observers)
        wb_b: Blue white balance 50-150 (IGNORED for observers)
        session_id: Client session ID - REQUIRED for authentication
    """
    try:
        if not camera.is_initialized:
            raise HTTPException(status_code=400, detail="Camera not connected")

        # STRICT AUTHENTICATION: session_id is required
        if not session_id:
            raise HTTPException(
                status_code=401,
                detail="Authentication required: session_id parameter missing",
            )

        # Get and validate session
        session = session_manager.get_session(session_id)
        if not session:
            raise HTTPException(status_code=401, detail="Invalid session_id")

        # Update session activity
        session_manager.update_session_activity(session_id)

        if session.is_master:
            # MASTER: Can change parameters and control hardware
            # Convert milliseconds to microseconds
            exposure_us = exp * 1000

            # Use new master capture method with white balance
            jpeg_bytes = camera.capture_fast_master(exposure_us, gain, wb_r, wb_b)

            # Broadcast parameter change to observers
            params_update = {
                "exposure_ms": exp,
                "gain": gain,
                "master_session": session_id,
            }
            if wb_r is not None:
                params_update["wb_r"] = wb_r
            if wb_b is not None:
                params_update["wb_b"] = wb_b

            await broadcast_parameter_update(params_update)

        else:
            # OBSERVER: Use master's parameters, cannot change anything
            jpeg_bytes = camera.capture_fast_observer()

        if jpeg_bytes:
            # Return raw JPEG bytes with proper content type
            return Response(
                content=jpeg_bytes,
                media_type="image/jpeg",
                headers={
                    "Cache-Control": "no-cache, no-store, must-revalidate",
                    "Pragma": "no-cache",
                    "Expires": "0",
                },
            )
        else:
            raise HTTPException(status_code=500, detail="Capture failed")

    except HTTPException:
        raise
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Capture failed: {e}")


@fastapi_app.get("/api/sessions")
async def get_sessions():
    """Get information about all client sessions."""
    try:
        return session_manager.get_session_info()
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Failed to get session info: {e}")


@fastapi_app.post("/api/request-master")
async def request_master(request: Request):
    """Request master control transfer."""
    try:
        data = await request.json()
        from_session = data.get("from_session_id")
        to_session = data.get("to_session_id")

        if not to_session:
            raise HTTPException(status_code=400, detail="Missing to_session_id")

        # For now, auto-approve all requests
        success = session_manager.request_master_transfer(from_session, to_session)

        if success:
            # Broadcast session update
            await broadcast_session_update()

            return {
                "status": "success",
                "message": "Master control transferred",
                "new_master": to_session,
            }
        else:
            raise HTTPException(status_code=400, detail="Master transfer failed")

    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Master request failed: {e}")


@fastapi_app.post("/api/force-master")
async def force_master(request: Request):
    """Force master control (admin override)."""
    try:
        data = await request.json()
        session_id = data.get("session_id")

        if not session_id:
            raise HTTPException(status_code=400, detail="Missing session_id")

        success = session_manager.force_master_transfer(session_id)

        if success:
            # Broadcast session update
            await broadcast_session_update()

            return {
                "status": "success",
                "message": "Master control force transferred",
                "new_master": session_id,
            }
        else:
            raise HTTPException(status_code=400, detail="Force master transfer failed")

    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Force master failed: {e}")


@fastapi_app.get("/api/latest-frame")
async def get_latest_frame():
    """Get the latest cached frame (stateless, no session required)."""
    try:
        if not camera.is_initialized:
            raise HTTPException(status_code=400, detail="Camera not connected")

        frame_bytes = camera.get_latest_frame()

        if frame_bytes:
            return Response(
                content=frame_bytes,
                media_type="image/jpeg",
                headers={
                    "Cache-Control": "no-cache, no-store, must-revalidate",
                    "Pragma": "no-cache",
                    "Expires": "0",
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
        if not camera.is_initialized:
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
    """Get information about the latest frame and background capture status."""
    try:
        if not camera.is_initialized:
            raise HTTPException(status_code=400, detail="Camera not connected")

        frame_info = camera.get_frame_info()
        current_settings = camera.get_current_settings()

        return {
            **frame_info,
            "current_settings": current_settings,
            "timestamp": time.time(),
        }

    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Frame info failed: {e}")


@fastapi_app.post("/api/start-background")
async def start_background_capture():
    """Start background capture thread."""
    try:
        if not camera.is_initialized:
            raise HTTPException(status_code=400, detail="Camera not connected")

        success = camera.start_background_capture()

        if success:
            return {
                "status": "success",
                "message": "Background capture started",
                "timestamp": time.time(),
            }
        else:
            raise HTTPException(
                status_code=500, detail="Failed to start background capture"
            )

    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Background start failed: {e}")


@fastapi_app.post("/api/stop-background")
async def stop_background_capture():
    """Stop background capture thread."""
    try:
        camera.stop_background_capture()

        return {
            "status": "success",
            "message": "Background capture stopped",
            "timestamp": time.time(),
        }

    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Background stop failed: {e}")


@fastapi_app.websocket("/ws/{client_ip}")
async def websocket_endpoint(websocket: WebSocket, client_ip: str):
    """WebSocket endpoint for real-time client communication."""
    await websocket.accept()

    # Check for force_master in query params
    query_params = parse_qs(
        str(websocket.url).split("?")[-1] if "?" in str(websocket.url) else ""
    )
    force_master = query_params.get("force_master", ["false"])[0].lower() == "true"

    # Create session
    session_id = session_manager.create_session(websocket, client_ip, force_master)

    try:
        # Send initial session info
        session_info = session_manager.get_session_info()
        await websocket.send_text(
            json.dumps(
                {
                    "type": "session_created",
                    "session_id": session_id,
                    "is_master": session_manager.is_master(session_id),
                    "session_info": session_info,
                }
            )
        )

        while True:
            try:
                # Wait for messages from client
                message = await websocket.receive_text()
                data = json.loads(message)

                message_type = data.get("type")

                if message_type == "heartbeat":
                    # Update heartbeat
                    session_manager.update_session_heartbeat(session_id)
                    await websocket.send_text(
                        json.dumps({"type": "heartbeat_ack", "timestamp": time.time()})
                    )

                elif message_type == "request_master":
                    # Handle master control request
                    current_master = session_manager.get_master_session()
                    if current_master:
                        # For now, auto-approve
                        session_manager.request_master_transfer(
                            current_master.session_id, session_id
                        )

                        # Broadcast session update to all clients
                        await broadcast_session_update()

                elif message_type == "parameter_change":
                    # Handle parameter changes from master
                    if session_manager.is_master(session_id):
                        # Broadcast parameter change to observers
                        await broadcast_parameter_update(data.get("parameters", {}))

            except WebSocketDisconnect:
                break
            except Exception as e:
                print(f"WebSocket message error: {e}")

    except WebSocketDisconnect:
        pass
    finally:
        # Clean up session
        session_manager.remove_session(session_id)
        await broadcast_session_update()


async def broadcast_session_update():
    """Broadcast session info update to all connected clients."""
    session_info = session_manager.get_session_info()
    message = json.dumps({"type": "session_update", "session_info": session_info})

    # Send to all connected WebSockets
    for session in session_manager.sessions.values():
        if session.websocket:
            try:
                await session.websocket.send_text(message)
            except Exception as e:
                print(f"Failed to broadcast to session {session.session_id}: {e}")


async def broadcast_parameter_update(parameters: dict):
    """Broadcast parameter changes to observer clients."""
    message = json.dumps(
        {"type": "parameter_update", "parameters": parameters, "timestamp": time.time()}
    )

    # Send to observer sessions only
    for session in session_manager.sessions.values():
        if session.websocket and not session.is_master:
            try:
                await session.websocket.send_text(message)
            except Exception as e:
                print(
                    f"Failed to broadcast parameters to session {session.session_id}: {e}"
                )


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

        # Start background capture for stateless frame delivery
        print("Starting background capture...")
        if camera.start_background_capture():
            print("Background capture started - stateless frame access now available!")
            print("New endpoints:")
            print("  - Stateless frame: http://localhost:8000/api/latest-frame")
            print("  - Update settings: POST http://localhost:8000/api/update-settings")
            print("  - Frame info: http://localhost:8000/api/frame-info")
        else:
            print("Warning: Background capture failed to start")
    else:
        print("Warning: Failed to connect to camera. Use /api/connect to retry.")

    # Start Flask in a separate thread
    flask_thread = threading.Thread(target=run_flask, daemon=True)
    flask_thread.start()

    # Start FastAPI in the main thread
    print("Starting web servers...")
    print("Flask (web pages): http://localhost:5000")
    print("FastAPI (API): http://localhost:8000")
    print(
        "Fast capture endpoint: http://localhost:8000/api/capture-fast?exp=100&gain=250"
    )
    print("Force master mode: http://localhost:5000/?force_master=true")
    run_fastapi()


if __name__ == "__main__":
    start_web_servers()
