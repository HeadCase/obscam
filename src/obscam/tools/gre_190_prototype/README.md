# GRE-190 browser delivery prototype

**Throwaway evidence tooling.** This package validates the shared measurement
contract, bounded latest-frame fan-out, JPEG/WebSocket finalist, multipart MJPEG
control, and real-browser presentation instrumentation. It is not production
delivery code.

Run read-only capability inspection:

```bash
python -m obscam.tools.gre_190_prototype preflight
```

Run the deterministic full-resolution source and open port 8190 in actual Safari
and Chromium browsers:

```bash
python -m obscam.tools.gre_190_prototype serve --fps 10 --quality 80
```

The generated source is only an instrumentation check. Final GRE-190 evidence
must replace it with the GRE-183 native RAW8 latest-frame handoff and must run on
the deployed Pi. Hardware H.264/WebRTC is a hard gate: preflight must report
`h264_webrtc_ready: true`; software H.264 is not an acceptable substitute.

The WebSocket wire format is a four-byte network-order JSON length, the UTF-8
`FrameEnvelope`, then one complete JPEG. Every client independently waits for the
current generation, so a slow client skips replaced frames without queueing or
delaying any peer.

Browser reports currently carry intentionally-invalid clock uncertainty. A
clock-offset calibration exchange must be completed before interpreting
cross-machine exposure-to-visible latency. Receive-to-visible and unique visible
cadence remain valid instrumentation checks.

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
