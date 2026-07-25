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

