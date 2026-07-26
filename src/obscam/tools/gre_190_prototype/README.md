# GRE-190 browser delivery prototype

**Throwaway evidence tooling.** This package validates the shared measurement
contract, bounded latest-frame fan-out, JPEG/WebSocket finalist, multipart MJPEG
control, and real-browser presentation instrumentation. It is not production
delivery code.

Run read-only capability inspection:

```bash
python -m obscam.tools.gre_190_prototype preflight
python -m obscam.tools.gre_190_prototype hardware-h264 --duration-s 30
```

Run the deterministic full-resolution source and open port 8190 in actual Safari
and Chromium browsers:

```bash
python -m obscam.tools.gre_190_prototype serve --fps 10 --quality 80
```

The generated source is only an instrumentation check. For the native dual-path
run, build the Rust backend, start MediaMTX as described below, then run:

```bash
cargo build --manifest-path \
  src/obscam/tools/gre_190_prototype/rust_backend/Cargo.toml
python -m obscam.tools.gre_190_prototype serve --native \
  --fps 20 --exposure-us 10000 --gain 0
```

For GRE-196, the native JPEG consumer sends every captured RAW8 generation to
independent neutral colour and monochrome pipelines. Compare them in separate
browser tabs by adding `treatment=colour` or `treatment=mono` to the query
string.

The Rust process exclusively owns the enrolled camera and fixed RAW8 buffer
pool. It independently feeds hardware H.264/RTSP and software JPEG encoders;
Python receives only framed, compressed JPEG packets and relays the newest one
to browsers. Software H.264 is not an acceptable substitute.

The WebSocket wire format is a four-byte network-order JSON length, the UTF-8
`FrameEnvelope`, then one complete JPEG. Every client independently waits for the
current generation, so a slow client skips replaced frames without queueing or
delaying any peer.

Browser reports currently carry intentionally-invalid clock uncertainty. A
clock-offset calibration exchange selects the lowest-uncertainty of nine samples.
Frame envelopes carry calibrated Unix timestamps for the native prototype's
cross-machine latency measurement. The duplicate `*_ns` fields retain the
original generated-source contract shape; they must not be interpreted as a
separate monotonic clock in native runs.

## Hardware H.264 WebRTC smoke test

The successful browser-delivery spike uses the checked-in `mediamtx.yml` with a
standalone MediaMTX v1.18.2 binary. It intentionally binds isolated ports and
must not be installed as a service. From the repository root, start the gateway:

```bash
mediamtx src/obscam/tools/gre_190_prototype/mediamtx.yml
```

In another shell, publish one hardware-encoded 1920x1080/10 fps test pattern:

```bash
ffmpeg -hide_banner -loglevel warning -re \
  -f lavfi -i testsrc2=size=1920x1080:rate=10 \
  -pix_fmt yuv420p -c:v h264_v4l2m2m -profile:v 578 \
  -b:v 8M -g 10 -f rtsp -rtsp_transport tcp \
  rtsp://127.0.0.1:18554/gre190
```

Open `http://PI_LAN_IP:18889/gre190/`. UDP ICE remains enabled, but TCP ICE on
port 18190 is required for the tested WireGuard client topology. The MediaMTX
API and metrics endpoints bind to loopback ports 19997 and 19998 respectively.
This smoke test proves browser delivery and shared fan-out, not end-to-end
latency; use timestamped browser instrumentation for latency evidence.

## Autonomous remote-browser run

Start the server on the Pi, choose one run ID, then open URLs of this form on the
remote laptop through WireGuard:

```text
http://PI_WIREGUARD_IP:8190/?auto=1&run=RUN_ID&client=safari-1&duration=60
http://PI_WIREGUARD_IP:8190/?auto=1&run=RUN_ID&client=safari-2&duration=60
http://PI_WIREGUARD_IP:8190/?auto=1&run=RUN_ID&client=chromium-1&duration=60
http://PI_WIREGUARD_IP:8190/?auto=1&run=RUN_ID&client=chromium-2&duration=60
```

Keep every tab foreground-visible until it says `COMPLETE`. Each client performs
nine clock-calibration samples, runs JPEG/WebSocket delivery, deliberately
reconnects halfway through, and uploads one validated result. Inspect all stored
clients at `GET /api/runs/RUN_ID`. Results are held only in memory and disappear
when the prototype server stops.
