# GRE-183 native full-resolution capture evidence

## Status

**Complete for GRE-183.** The deployed ASI662MC was measured at SuperSpeed on
the production Raspberry Pi while the ASI178MC remained connected and in AllSky
production use. The evidence covers the full production exposure range
(10 ms-30 s), the full gain range (0-600), SDK video and snapshot acquisition,
format/control screening, setting interruption, repeated finalists, and a
30-minute sustained Rust run.

Long-duration two-camera identity and unplug/replug recovery remain GRE-191
work. Browser delivery is GRE-190, and the production architecture decision is
GRE-188.

## Decision

Use **SDK video acquisition for the entire 10 ms-30 s range**. There is no
data-derived cadence crossover: video was faster than snapshot at every matched
exposure. Do not carry the legacy 200 ms boundary into the greenfield design.

Use a native hot path when near-ceiling RAW8 cadence matters. Corrected Python
reached 92.68 fps in the 60-second finalist, versus 99.43 fps for C and
99.35 fps for Rust, and used materially more CPU and memory. Rust matched the C
ceiling closely and its bounded latest-frame pipeline degraded gracefully in a
30-minute run, so Rust is a viable implementation language. It did not show an
intrinsic speed advantage over C; GRE-188 should choose Rust only if its
ownership, recovery, and integration benefits justify it.

Acquire color data as RAW8 and debayer downstream. At the fastest screened
point, RAW8 achieved about 99.4 fps in C/Rust. SDK RGB24 and Y8 were slower and
reported substantially more SDK drops.

## Compared paths

- Native C reference: one guarded caller-owned buffer, SDK calls directly from
  the capture loop, and full-buffer zlib CRC32.
- Corrected python-zwoasi: one reused `bytearray`, direct `get_video_data()`
  without per-frame ROI lookup or full-frame allocation, followed by the same
  CRC32.
- Rust pipeline: four guarded buffers, a dedicated capture thread, a bounded
  latest-frame slot, and in-place CRC32 consumption without a downstream copy.

All paths used native 1920x1080 frames and emitted the same validated JSON
contract. Exact duplicate images were separated from corruption and transport
signals because a clipped sensor legitimately produces identical frames.

Snapshot and transition measurements used the native C reference so the result
measures SDK acquisition behavior without adapter overhead. The language
comparison used SDK video mode across all three runners.

## Production environment and safety

- Raspberry Pi 4, aarch64, four Cortex-A72 CPUs, 4 GB RAM
- ZWO SDK `1, 38, 0, 0`
- ASI662MC `03c3:662b`, native 1920x1080, negotiated at 5000 Mbit/s
- ASI178MC `03c3:178a`, AllSky production camera, remained present at
  480 Mbit/s throughout completed cells
- Advertised ASI662MC outputs: RAW8, RGB24, Y8
- SDK gain limits: 0-600; exposure limits exceed the tested product range
- `ASI_BANDWIDTHOVERLOAD`: 40-100; `ASI_HIGH_SPEED_MODE`: 0-1
- AllSky, rclone, and WireGuard activity remained present

Discovery selected the ASI662MC by model before opening a camera. Native runners
also inspected properties before opening only the exact ASI662MC match. A
100 ms sysfs sentinel terminated a runner if the ASI178MC disappeared. The
prototype never stopped AllSky or power-cycled, reset, or opened the ASI178MC.

## USB 3 screen

The screen ran 99 five-second cells: three runners, three formats, representative
exposures, bandwidth 40/50/100, and high-speed mode 0/1. It recorded zero capture
errors, zero corrupt frames, zero Rust latest-slot drops, no sentinel event, and
no thermal throttling. SDK drop-counter deltas totalled 2,400 for C, 3,286 for
Python, and 1,951 for Rust across the whole screen; these were concentrated in
format/control combinations that could not consume the camera stream cleanly.

The fastest requested point was 10 ms, high-speed 1, bandwidth 100:

| Format | Native C fps / SDK drops | Python fps / SDK drops | Rust fps / SDK drops |
| --- | ---: | ---: | ---: |
| RAW8 | 99.396 / 0 | 91.685 / 37 | 99.432 / 0 |
| RGB24 | 41.568 / 173 | 25.735 / 254 | 56.434 / 99 |
| Y8 | 36.472 / 315 | 29.507 / 351 | 38.971 / 303 |

The measured ceiling is about 99.4 full 1920x1080 RAW8 frames per second. The
higher SDK-advertised maximum was not observed with identical full-buffer CRC32
work and should not be used as a production promise.

## 60-second RAW8 finalists

All finalists requested 10 ms, gain 0, high-speed 1, and bandwidth 100. None
reported a capture error, corrupt frame, pipeline replacement, or thermal
throttle.

| Path | fps | Capture p50/p95/p99/max ms | SDK drops | CPU | Peak RSS | Peak temp |
| --- | ---: | --- | ---: | ---: | ---: | ---: |
| Native C | 99.427 | 8.002 / 8.583 / 9.568 / 13.936 | 0 | 78.2% | 19.5 MiB | 68.7 C |
| Python | 92.682 | 2.971 / 4.318 / 5.162 / 10.429 | 403 | 135.1% | 61.0 MiB | 74.0 C |
| Rust | 99.355 | 10.036 / 10.666 / 12.340 / 21.351 | 5 | 80.7% | 25.8 MiB | 73.0 C |

Python's short SDK-call latency reflects reading already-buffered frames; its
client-visible cadence and SDK drop count show that the serial Python/CRC path
did not keep up with the native ceiling.

## Video versus snapshot over 10 ms-30 s

Matched native C RAW8 cells used gain 0, bandwidth 50, and high-speed 0. All 22
cells had zero SDK drops, capture errors, corrupt frames, and throttling.

| Exposure | Video fps / p50 ms | Snapshot fps / p50 ms | Snapshot overhead |
| --- | ---: | ---: | ---: |
| 10 ms | 76.324 / 11.084 | 3.815 / 257.984 | 246.900 ms |
| 25 ms | 39.865 / 21.767 | 3.646 / 270.795 | 249.028 ms |
| 50 ms | 20.007 / 47.766 | 3.319 / 295.803 | 248.037 ms |
| 100 ms | 9.986 / 98.041 | 2.885 / 342.458 | 244.416 ms |
| 200 ms | 4.999 / 198.068 | 2.219 / 447.053 | 248.985 ms |
| 500 ms | 1.998 / 497.682 | 1.327 / 747.561 | 249.878 ms |
| 1 s | 0.981 / 1016.466 | 0.797 / 1251.174 | 234.708 ms |
| 2 s | 0.492 / 2030.697 | 0.441 / 2265.371 | 234.675 ms |
| 5 s | 0.199 / 5030.148 | 0.190 / 5268.975 | 238.826 ms |
| 10 s | 0.100 / 10027.092 | 0.097 / 10263.862 | 236.770 ms |
| 30 s | 0.03330 / 30031.554 | 0.03304 / 30269.009 | 237.455 ms |

Snapshot adds a nearly fixed 235-250 ms per exposure. Its relative penalty
shrinks at long exposures, but it never overtakes video. The simpler production
default is therefore one video acquisition path over the full range. GRE-194's
matched nighttime experiment subsequently confirmed that video and snapshot
frames follow the same exposure-response curve: at 30 seconds and gain 400,
their average full-frame RAW8 signal differed by only 0.168%, less than the
repeat-to-repeat variation. Video mode therefore performs a genuine 30-second
integration and snapshot mode has no separate image-integration role.

## Gain envelope and image-content classification

RAW8 video anchors at 10 ms, 100 ms, 1 s, and 30 s covered gains 0, 100, 250,
400, and 600. All 20 anchors had zero SDK drops, capture errors, corruption, and
throttling. Gain did not materially change acquisition cadence.

The daytime scene clipped increasingly early as gain rose:

| Gain | Saturated pixels at 10 ms | 100 ms | 1 s | 30 s |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 1.66% | 15.12% | 91.43% | 99.50% |
| 100 | 4.39% | 51.24% | 100% | 100% |
| 250 | 30.11% | 99.89% | 100% | 100% |
| 400 | 92.58% | 100% | 100% | 100% |
| 600 | 100% | 100% | 100% | 100% |

A one-variable diagnostic at 200 ms changed gain 250 to gain 0 and restored
25/25 unique frames at 4.996 fps. Identical CRCs at high gain/exposure are thus
sensor clipping in this scene, not stale-frame transport. Production health
must combine cadence/drop/error signals with image statistics; it must not treat
duplicate pixels alone as camera failure.

## Exposure transitions

Three native C trials measured both live application and explicit cancellation:

| Transition | Apply/restart | First attributable new frame | Detail |
| --- | ---: | ---: | --- |
| Video 10 ms -> 100 ms, live | 6.064 ms | 113.027 ms | one queued old frame; new capture call 100.525 ms |
| Snapshot 30 s -> 100 ms, cancel | 134.816 ms | 481.127 ms | new snapshot acquisition 346.311 ms |
| Video 30 s -> 100 ms, stop/restart | 303.709 ms | 649.061 ms | new video acquisition 345.352 ms |

Snapshot cancellation was 167.934 ms faster in this worst-direction trial, but
snapshot's fixed steady-state cost is larger and applies to every frame. Prefer
video throughout, with an explicit stop/apply/restart generation boundary when
interrupting a long exposure. Discard pre-boundary frames rather than relying on
timing heuristics or a hard-coded exposure crossover.

## 30-minute Rust sustained result

The production-shaped Rust pipeline ran RAW8 video at 10 ms, gain 0, high-speed
1, bandwidth 100 for 1,800.001 seconds:

- 178,895 frames at 99.386 fps; 178,880 reached the CRC consumer
- 79 SDK drops and 15 bounded latest-frame replacements
- one recoverable capture error, zero corrupt frames, no restart or early exit
- capture p50/p95/p99/max: 10.033/10.907/12.758/27.041 ms
- inter-frame p50/p95/p99/max: 10.049/10.925/12.781/524.743 ms
- 83.0% observed CPU, 30.1 MiB peak RSS, 79.4 C peak, no throttle
- ASI662MC and ASI178MC were both present before and after

The 525 ms tail coincided with a noisy production USB environment that logged
resets. The bounded pipeline recovered and resumed ceiling cadence. RSS growth
within this prototype includes retaining per-frame timing samples for the final
artifact; a production implementation should use bounded online histograms.

## Recommendation to GRE-188

1. Put SDK capture, frame ownership, generation boundaries, and bounded
   latest-frame delivery in a small native module/worker. Keep orchestration and
   low-rate control outside the hot loop.
2. Rust is a sound candidate: it matched C throughput and provides a natural
   ownership model for guarded buffers. Choose it for maintainability and
   recovery correctness, not for a claimed speedup over C.
3. Use RAW8 video across 10 ms-30 s. Debayer where needed downstream; do not use
   SDK RGB24/Y8 on the latency-sensitive path based on this screen.
4. On a setting change, establish a generation boundary, explicitly restart for
   long-exposure interruption when responsiveness requires it, and drop stale
   pre-generation work.
5. Expose drops, capture errors, latency tails, clipping, temperature, and
   throttling independently. Graceful latest-frame replacement is preferable to
   queue growth.

## Limits and follow-up ownership

This ticket does not establish browser-visible latency, multi-client behavior,
degraded network/VPN delivery, hours-long coexistence, camera identity after
re-enumeration, or physical disconnect/replug recovery. The frequent ASI178MC
kernel resets observed while AllSky continued running are a shared-host
reliability signal for GRE-191, not a reason to open or disturb that production
camera in GRE-183.

Raw artifacts are under [`results/`](results/). Each contains the common result,
per-second resource samples, and before/after host snapshots.
