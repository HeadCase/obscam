# GRE-203 full-resolution grayscale demosaicing qualification

This directory records the qualification of a RAW8-derived, Bayer-aware
grayscale treatment on the exact ASI662MC. The candidate reconstructs RGB at
each 1920×1080 RGGB sample with fixed bilinear interpolation, immediately
converts it to full-range BT.601 luminance, and writes neutral YUV420 chroma.
It allocates no intermediate RGB frame and retains no temporal image history.

The qualification-only `grayscale_demosaiced` treatment is bounded to the
`gre200-probe` evidence harness. The existing `mono` treatment remains the
control that copies the colour-filter mosaic directly into H.264 luma. GRE-207
will select the production treatment and remove the rejected path.

## Deterministic control

The Rust tests construct a constant-colour RGGB field whose mosaic-copy luma
contains a deterministic 20–200 DN two-by-two grid. The demosaiced candidate
must produce the same neutral luminance at every output pixel. A separate
step-edge control verifies native dimensions, monotonicity, and retained
full-frame contrast.

## Matched deployed protocol

Run both `mono` and `grayscale_demosaiced` through the real Rust owner, shared
`h264_v4l2m2m` encode, MediaMTX WHEP/WebRTC relay, and foreground-visible
Safari and Chromium sessions. Use the GRE-205 settings:

- short/high-gain: 50 ms, gain 500
- long/low-gain: 1 s, gain 100

For each scene and treatment, retain exact correlated samples and record:

- Bayer-grid visibility and fine telescope-edge detail
- unique browser-presented cadence
- exposure-end-to-visible latency and clock uncertainty
- processing and total-stack CPU, RSS, and Pi temperature
- one-viewer and four-visible-viewer behavior
- output dimensions, source generation, settings generation, treatment,
  runtime epoch, stream epoch, and browser RTP timestamp

All measurements use full 1920×1080 RAW8 acquisition and the single shared
hardware-H.264 media path. Binning, scaling, ROI reduction, rotation, disk
frames, a second media path, and per-viewer encoding are forbidden.

## Evidence status

Local deterministic and contract verification passed on 2026-07-28. The full
workspace gate passed with 25 Rust tests after the optimized path was added;
browser contract checks, formatting, Clippy with warnings denied, `cargo deny`,
`cargo machete`, and `git diff --check` also passed.

### Deployed-Pi measurements

The exact ASI662MC and the isolated shared H.264/MediaMTX path were exercised on
the deployed Pi. The first reference implementation was rejected during the
run: it consumed about 116% of one core in the Rust owner. A mathematically
equivalent interior fast path reduced that to about 40% while retaining the
reference bilinear result byte-for-byte in deterministic tests.

Matched 30-second, 50 ms/gain 500 samples with no viewers measured:

| Treatment | Rust CPU | FFmpeg CPU | Rust RSS | Temperature |
| --- | ---: | ---: | ---: | ---: |
| mosaic-copy `mono` | 28.28% of one core | 10.86% | 32.2 MiB | 75.9→74.0°C |
| `grayscale_demosaiced` | 39.93% of one core | 10.12% | 32.1 MiB | 77.9→75.0°C |

With one live WebRTC reader, the 50 ms/gain 500 candidate completed 600 unique
source generations in 30.032 seconds, with 38.63% Rust CPU, 10.16% FFmpeg CPU,
32.4 MiB Rust RSS, and temperature falling from 71.5°C to 71.1°C.

With one live reader at 1 s/gain 100, the candidate completed 29 source
generations in 30.045 seconds, with 3.26% Rust CPU, 1.50% FFmpeg CPU, 32.3 MiB
Rust RSS, and temperature falling from 73.5°C to 70.6°C.

Matched four-viewer, 50 ms/gain 500 samples measured:

| Treatment | Unique source cadence | Combined CPU | Stack RSS | Temperature |
| --- | ---: | ---: | ---: | ---: |
| mosaic-copy `mono` | 900 / 45.053 s (19.98 fps) | 50.56% of one core | 180.45 MiB | 58.9→59.9°C |
| `grayscale_demosaiced` | 900 / 45.039 s (19.98 fps) | 55.48% of one core | 180.13 MiB | 59.4→60.3°C |

All four foreground-visible Safari/Chromium sessions remained connected. The
candidate normally displayed about 210–240 ms exposure-end-to-visible latency,
showed honest `grayscale_demosaiced` metadata with exact frame correlation, and
had no visible Bayer mosaic. The mosaic-copy control made the grid visible.

A later multi-second latency excursion affected the mosaic control and then
recovered with the demosaiced treatment at the same encoder bitrate. Host
measurements identified concurrent ASIAir-to-NAS rclone/NFS traffic saturating
the shared physical interface; capture remained exactly 20 fps. The excursion
therefore does not indicate a demosaicing regression. Bounded agent-readable
browser latency aggregation is separated into GRE-208.

The firmware throttle word began at `0x80000` and later read `0xe0000`; both
contain historical flags only. No current-condition bit appeared during a
GRE-203 sample. MediaMTX reported the H.264 path ready throughout.

## Decision

Qualify full-resolution grayscale demosaicing for production selection in
GRE-207. It removes the visible Bayer grid and improves useful image quality at
the native 1920×1080 resolution. Its four-viewer cost is about five additional
CPU percentage points, with no cadence, RSS, thermal, media-path, or observed
normal-latency regression. Native Y8 remains rejected by GRE-205; GRE-207 should
select this RAW8-derived treatment and remove the mosaic-copy control.
