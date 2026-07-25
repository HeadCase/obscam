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
uses FFmpeg only as a thin owner of the hardware encoder and retains GStreamer
for H.264 parsing, RTP/WebRTC, and fMP4 packaging. Software H.264 remains
forbidden.
