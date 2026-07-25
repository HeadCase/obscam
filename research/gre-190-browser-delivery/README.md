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

