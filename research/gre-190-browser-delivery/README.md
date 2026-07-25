# GRE-190 browser delivery evidence

This directory records evidence from the throwaway GRE-190 browser-delivery
prototype. Generated-source runs validate instrumentation and client isolation;
they do not decide the production delivery architecture.

## 2026-07-25 four-client WireGuard instrumentation run

Run ID: `gre190-wg-generated-20260725-r2`

The Raspberry Pi served full-resolution 1920x1080 JPEG frames at a requested 10
fps. A macOS laptop opened two Safari 26.5.2 windows and two Brave/Chromium 149
windows. The client used `192.168.1.200`; the Pi observed source `10.164.190.3`,
confirming that the laptop's WireGuard route carried the traffic. All windows
remained browser-visible and each connection was deliberately closed for one
second and re-established halfway through the 60-second run.

| Client | Presented | Cadence | Skipped generations | Receive-to-visible p50/p95/p99/max |
| --- | ---: | ---: | ---: | --- |
| Brave 1 | 592 | 9.862 fps | 9 | 10.4 / 14.1 / 16.8 / 18.8 ms |
| Brave 2 | 590 | 9.830 fps | 10 | 11.7 / 15.2 / 18.2 / 21.8 ms |
| Safari 1 | 589 | 9.812 fps | 10 | 14 / 19 / 22 / 24 ms |
| Safari 2 | 590 | 9.827 fps | 10 | 15 / 17 / 20 / 27 ms |

All received frames were uniquely presented. All clients reported one reconnect,
zero hidden-frame events, and zero errors. Maximum presentation gaps were
1.065--1.150 seconds and correspond to the intentional one-second reconnect.
Brave median JPEG decode time was 9--10 ms; Safari median decode time was 13--15
ms. No slow client caused unbounded server queueing because every connection
read from the same bounded latest-generation fan-out.

Clock-calibration uncertainty was 11.7--12.65 ms. The current generated frame
envelope uses server monotonic timestamps while browser events use Unix time, so
this run supports browser receive-to-visible latency and visible cadence only.
It must not be used as exposure-end-to-visible evidence.

The first run ID without the `-r2` suffix is invalid: Uvicorn lacked a WebSocket
protocol dependency, and all clients correctly uploaded connection failures.
The missing dependency was added before this successful run.

## Hardware H.264 preflight

The Pi exposes the Broadcom `bcm2835-codec-encode` V4L2 M2M device at
`/dev/video11`. GStreamer's `v4l2h264enc` negotiated successfully but failed to
start streaming for every tested I/O mode and resolution; the kernel reported
`bcm2835_codec_start_streaming: Failed enabling i/p port, ret -3`. The failure
also occurred at 640x480 and was not caused by memory pressure, codec ownership,
or disabled firmware: H.264 was enabled, no process owned `/dev/video11`, and
CMA had approximately 500 MiB free.

FFmpeg's `h264_v4l2m2m` wrapper against the same `/dev/video11` hardware passed.
A 30-second deterministic 1920x1080/10 fps run encoded 300/300 frames into an
18,010,669-byte Annex-B H.264 stream. Temperature rose from 56.9 C to 60.8 C and
`throttled=0x0` remained clear. A subsequent invocation of the codified gate
encoded 20/20 full-resolution frames in two seconds. The prototype therefore
uses FFmpeg only as a thin owner of the hardware encoder. Software H.264 remains
forbidden.

## Hardware H.264 WebRTC delivery over WireGuard

A direct custom GStreamer `webrtcbin` sender was rejected after repeated browser
negotiations produced no visible media. The useful negative evidence was that
hand-written signaling and per-peer pipeline management added substantial
diagnostic surface without proving delivery. The prototype then pivoted to an
isolated MediaMTX v1.18.2 gateway; it was not installed as a service and did not
modify the host's existing MediaMTX instance.

The successful path was one FFmpeg `h264_v4l2m2m` hardware encode of a
1920x1080/10 fps test pattern, published over loopback RTSP/TCP to MediaMTX and
delivered as H.264 WebRTC/WHEP. Brave 149 and Safari 26.5.2 on the macOS laptop
displayed the moving pattern concurrently through the WireGuard route. MediaMTX
reported both peer connections established and reading the same H.264 track.

UDP-only ICE did not establish across this client/VPN topology. Both successful
sessions selected MediaMTX's TCP ICE candidate
`host/tcp/192.168.1.200/18190`, with peer-reflexive client candidates on
`10.164.190.3`. At the evidence snapshot the two sessions had sent 48,700 and
24,628 RTP packets respectively, with zero outbound frames discarded. The
shared path had zero inbound frame errors. Pi temperature was 56.9 C and
`throttled=0x0`; MediaMTX used approximately 58 MiB RSS and 12.3% CPU, while
FFmpeg used approximately 105 MiB RSS and 14.1% CPU.

This proves browser compatibility, concurrent fan-out from one hardware encode,
and the required VPN transport fallback. It does not yet provide
exposure-end-to-visible latency: the gateway's built-in player has no frame
generation timestamp or presentation telemetry. That measurement must use the
instrumented browser harness with a timestamped camera source or equivalent
end-to-end marker.

## Native-camera Rust-to-WebRTC integration

The production-shaped Rust prototype reused GRE-191's fail-closed identity
boundary: it required exactly one exact `ZWO ASI662MC` match, opened only that
candidate, and validated factory serial `1d274e0920010900`. The ASI178MC AllSky
capture process remained running and both USB devices remained present after
identity and capture checks.

A fixed four-buffer Rust owner captured 1920x1080 RAW8 SDK video and exposed
independent latest-frame leases to logical JPEG and real H.264 consumers. In a
10-second 10 ms/gain-0 integration run, capture completed 914 frames (91.4 fps)
with zero pool starvation. The FFmpeg Bayer conversion plus
`h264_v4l2m2m` sink completed 224 generations while replacing 690 obsolete H.264
generations; the independent JPEG lease continued without being blocked. The
gateway received a valid 1920x1080 Baseline H.264 track with zero inbound frame
errors. This demonstrates bounded backpressure isolation rather than queueing
the camera's approximately 91 fps output behind the approximately 22 fps colour
H.264 processing ceiling.

Real ASI662MC frames were displayed in Brave through MediaMTX/WebRTC at both the
fast exposure point and closed-roof night settings. The latter exposed a
long-exposure transport requirement: 10-second integrations plus processing
slightly exceeded MediaMTX's default 10-second RTSP read timeout, causing the
publisher to be closed and FFmpeg to report a broken pipe. Raising the isolated
gateway's `readTimeout` to 45 seconds kept the same publisher and browser session
connected across repeated 10-second inter-frame gaps with increasing byte
counters and zero frame errors.

Closed-roof image cleanup is not part of GRE-190. Follow-up work is tracked in
GRE-196 (monochrome/spatial denoise), GRE-197 (defective pixels), and GRE-198
(temperature-aware dark correction).

## Native dual-path browser run

Run ID: `native-dual-20260726`

The Rust owner simultaneously fed FFmpeg's hardware H.264 publisher and a
single-thread, low-delay JPEG encoder from independent latest-frame leases. Its
only Python-facing media interface was a bounded stream of compressed JPEG plus
validated frame metadata; Python did not receive RAW8 frames or own capture.

A five-second direct integration captured 470 generations, completed 79 H.264
and 28 JPEG generations, and had zero buffer-pool starvation. All 28 emitted
JPEG packets had valid start/end markers and strictly increasing generations.

The same process then served a 30-second real-camera JPEG/WebSocket run to Brave
149 and Safari 26.5.2 over WireGuard. Both clients stayed visible, presented
every frame received, deliberately reconnected once, and reported no errors.

| Client | Presented | Cadence | Skipped capture generations | Encode-to-visible p50/p95 | Decode p50 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Brave | 203 | 6.77 fps | 2,380 | 165.1 / 364.2 ms | 8.0 ms |
| Safari | 178 | 5.93 fps | 2,367 | 195.1 / 408.9 ms | 13.0 ms |

Clock uncertainty was 19.5 ms in Brave and 13.0 ms in Safari. The high skipped
generation counts are intentional latest-frame replacement against the camera's
roughly 90 fps RAW8 cadence, not queued loss. This run demonstrates reliable
isolation and reconnection, but it also shows that full-resolution software JPEG
colour conversion on this Pi delivers only about 6--7 visible fps and materially
higher latency than its browser decode time.

### Decision

Proceed with the Rust media backend as the production candidate: one exact
camera owner, a fixed RAW8 pool, and independent bounded consumers. Use one
hardware H.264 encode with MediaMTX WebRTC/WHEP fan-out as the primary monitoring
path because it sustains substantially more native-camera generations and has
already displayed concurrently in Brave and Safari over the required VPN TCP
ICE fallback. Keep the compressed JPEG/WebSocket interface as the instrumented
fallback and diagnostic path, not the default live view. Do not pursue the
custom GStreamer WebRTC sender, software H.264, or per-client encoding.

The remaining uncertainty is quantitative H.264 exposure-to-visible latency:
the stock MediaMTX player exposes presentation callbacks but no source generation
metadata. Production implementation should add a narrow telemetry correlation
mechanism around the WHEP player and verify that metric without moving capture or
raw frames back into Python.
