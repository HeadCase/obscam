# GRE-207 production monochrome treatment

GRE-207 selects the full-resolution RAW8-derived grayscale treatment qualified
by GRE-203 as the production `mono` treatment. Rust reconstructs RGB at every
1920×1080 RGGB sample with fixed bilinear interpolation, immediately converts
that sample to full-range BT.601 luminance, and fills YUV420 chroma with 128.
The hot path allocates no intermediate RGB frame and retains no temporal image
history.

## Compared evidence

The decision compares the equivalent short/high-gain (50 ms, gain 500) and
long/low-gain (1 s, gain 100) observatory scenes from GRE-203 and GRE-205.
GRE-205 rejected the ASI662MC native Y8 candidate. The remaining GRE-203
comparison measured the selected treatment against direct RAW8 mosaic-copy
monochrome through the Rust owner, shared hardware H.264 encoder, MediaMTX, and
foreground-visible Safari and Chromium clients.

At 50 ms with no viewers, mosaic-copy used 28.28% of one core and the selected
treatment used 39.93%; both held about 32 MiB Rust RSS. With four viewers, both
delivered 900 unique generations at 19.98 fps. Combined stack CPU rose from
50.56% to 55.48% of one core, while stack RSS fell slightly from 180.45 MiB to
180.13 MiB. At 1 s with one viewer, the selected path used 3.26% Rust CPU and
32.3 MiB RSS. All four viewers remained connected, normal
exposure-end-to-visible latency was about 210–240 ms, and exact generation,
treatment, runtime-epoch, stream-epoch, and RTP correlation remained truthful.

## Decision

Retain one production `mono` treatment: the optimized, full-resolution
Bayer-aware conversion. It removes the visible Bayer grid while preserving
useful telescope-edge contrast at native resolution. Its measured CPU premium
is acceptable because it caused no cadence, RSS, thermal, media-path, or normal
latency regression.

Reject native Y8 because GRE-205 found it inferior on the exact ASI662MC.
Reject direct RAW8 mosaic-copy because its colour-filter grid remains visible.
The qualification-only `grayscale_demosaiced` contract value and the
mosaic-copy implementation are removed; callers continue to request `mono`.

## Reproduction evidence

Detailed protocol, deterministic controls, measurements, browser observations,
and the shared-interface latency excursion analysis remain in
`research/gre-203-grayscale-demosaicing/README.md`. The production selection
does not alter capture dimensions, spatial orientation, media delivery, runtime
state, or the exact-correlation model.
