# ObsCam

ObsCam is a small observatory camera monitor for remote browser-based viewing.
It runs a single FastAPI service that owns camera capture, serves the web UI, and
exposes endpoints for streaming, telemetry, settings updates, and snapshots.

The current codebase is optimized for simple, reliable operation on Raspberry Pi
class hardware. The implemented camera backends are ZWO ASI and a synthetic
fixture-backed camera for local development.

## Quick Start

Install dependencies and run the app from the repository root:

```bash
uv sync
uv run obscam
```

Open the UI at [http://localhost:8000](http://localhost:8000).

## Configuration

ObsCam is configured with environment variables.

```bash
# Camera backend: "zwo" or "synthetic"
export OBSCAM_CAMERA_BACKEND=synthetic

# Optional fixture directory for the synthetic camera
export OBSCAM_SYNTHETIC_FRAME_DIR=assets/test_loop

# Enable verbose logging
export OBSCAM_DEBUG=true

# Optional path to the ZWO SDK shared library
export ZWO_ASI_LIB=/usr/local/lib/libASICamera2.so
```

Notes:

- `OBSCAM_CAMERA_BACKEND` defaults to `zwo`.
- The synthetic backend is the simplest way to run the UI without camera hardware.
- Camera settings are cached to `/dev/shm` when available, otherwise under `/tmp/obscam`.

## What It Does

- Serves the browser UI and API from a single FastAPI process on port `8000`
- Streams the latest camera frames as MJPEG at `/stream.mjpg`
- Publishes frame and settings telemetry over Server-Sent Events at `/api/telemetry`
- Allows exposure and gain changes from the browser UI
- Saves snapshots under `assets/`
- Persists the latest settings asynchronously between runs
- Writes rotating logs under `logs/`

## Useful Endpoints

- `GET /` - browser UI
- `GET /api/bootstrap` - initial settings, capabilities, and stream URLs
- `GET /api/status` - backend and camera status
- `GET /api/latest-frame` - most recent JPEG frame
- `GET /api/frame-info` - current frame metadata and backend state
- `GET /stream.mjpg` - MJPEG stream
- `GET /api/telemetry` - Server-Sent Events telemetry stream
- `POST /api/settings` - queue exposure and gain changes
- `POST /api/snapshots` - save the latest buffered frame to disk
- `GET /api/connect` - retry backend startup if camera initialization failed

## Snapshots

`POST /api/snapshots` writes the latest buffered JPEG frame to `assets/`.

The request body accepts:

```json
{
  "subdirectory": "optional/subdir",
  "filename_prefix": "snapshot"
}
```

`subdirectory` must stay within `assets/`. Filenames are timestamped and made
safe for sorting and shell use.

## Logs

ObsCam writes logs to `logs/`:

- `logs/obscam.log` - main application log
- `logs/obscam-error.log` - error log
- `logs/obscam-debug.log` - debug log when `OBSCAM_DEBUG=true`

Useful commands:

```bash
tail -f logs/obscam.log
tail -f logs/obscam-error.log
```

## Development

Run the test suite:

```bash
uv run pytest
```

Run formatting and lint hooks:

```bash
uv run pre-commit install
uv run pre-commit run --all-files
```

## Repo Layout

- `src/obscam/api/` - FastAPI app and endpoints
- `src/obscam/core/` - backend service, capture loop, frame buffer, backend selection
- `src/obscam/camera/` - camera backends
- `src/obscam/storage/` - settings persistence
- `frontend/` - templates, JavaScript, and CSS
- `assets/` - fixture images and saved snapshots
- `tests/` - pytest coverage for synthetic camera and snapshot behavior
