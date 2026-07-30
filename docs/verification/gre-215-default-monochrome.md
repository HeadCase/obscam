# GRE-215 default monochrome media-path verification

## Implemented path

The continuously warm camera worker consumes the production `CameraSource`
contract, applies the default 10 ms/gain 100 settings, and converts each
validated 1920×1080 RAW8 RGGB generation directly to I420. Missing colour
samples use fixed bilinear interpolation at the native frame edges as well as
the interior. Full-range BT.601 integer coefficients (`77R + 150G + 29B`) form
the luma plane; both quarter-resolution chroma planes remain neutral 128. No
intermediate RGB frame or temporal image history exists.

One two-buffer mailbox separates processing from publication. It owns at most
one pending generation, overwrites pending obsolete work in place, and lets the
encoder discard a taken generation if newer work arrives before publication.
All processing and mailbox buffers are allocated once and reused.

Rust starts one long-lived FFmpeg child with native I420 input,
`h264_v4l2m2m`, 1.5 Mbps bitrate/maxrate/buffer, 20 fps input timing, a
20-frame maximum GOP, no B-frames, and one local RTSP publication at
`rtsp://127.0.0.1:8554/obscam`. There is no software encoder fallback.

`deploy/mediamtx.yml` enables only local TCP RTSP ingestion and WHEP/WebRTC
fan-out. RTMP, HLS, SRT, and MoQ are explicitly disabled. The compatibility
pin is MediaMTX v1.19.3 for Linux ARM64 with SHA-256
`b3b2b519420f24a1f262feccdfbee474c8bdedcbf318d5ec4d586f582dfacb00`.

The browser derives `scheme://page-host:8889/obscam/whep` from the validated
hostless runtime descriptor, negotiates a receive-only WebRTC track directly
with MediaMTX, and displays it in a muted `playsinline` video element with
`object-fit: contain`. Rust neither proxies WHEP nor handles media per viewer.

## Automated evidence

- Deterministic Bayer tests check native dimensions and I420 length, worked
  BT.601 values on all four Bayer sites, top-right/bottom-left/bottom-right
  edge reconstruction, neutral chroma, generation preservation, and processing
  buffer reuse. A direct RAW8 mosaic copy produces four unequal values and
  fails the uniform-patch assertion.
- Latest-only tests prove generations 1–3 collapse to one pending generation 3
  and prove an in-flight generation becomes explicitly obsolete when a newer
  generation arrives.
- The FFmpeg boundary test feeds one real native-size I420 generation through a
  fake external executable and checks the exact hardware codec, bitrate,
  timing/GOP, RTSP transport, publication path, and input byte count. A missing
  FFmpeg executable fails without fallback.
- TypeScript tests prove the media descriptor cannot replace the page
  authority. Rust HTTP contract tests prove the compiled video/WHEP assets and
  uncropped presentation rule are served.

## Deployed-Pi smoke — 2026-07-30

The test used the real Raspberry Pi 4 service process, distro FFmpeg 5.1.9,
real `h264_v4l2m2m`, pinned MediaMTX v1.19.3, and the explicit
`OBSCAM_CAMERA_SOURCE=deterministic` hardware-edge substitute.

- MediaMTX started only RTSP on `127.0.0.1:8554` and WHEP/WebRTC on ports 8889
  and 8189.
- MediaMTX observed one publisher and one H.264 track on path `obscam`.
- The runtime contract reported capture and encoder ready while retaining relay
  `not_observed` and `latestFrame: null`; no correlation facts were invented.
- The Mac browser reached `http://10.164.190.1:18080` over WireGuard and posted
  directly to `http://10.164.190.1:8889/obscam/whep`, receiving HTTP 201.
- Four simultaneous browser tabs each reached `readyState=4` with a decoded
  1920×1080 video track and the unavailable overlay hidden. MediaMTX recorded
  four independent readers of the same single H.264 track.
- Computed video presentation was `object-fit: contain`. The only console error
  was the pre-existing missing `/favicon.ico` noted by GRE-211.

Exact source-generation-to-browser-presentation correlation is intentionally
deferred to GRE-217. Until that contract exists, presentation does not claim
frame generation, age, cadence, or latency.
