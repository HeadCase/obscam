# Architecture Authority

Linear issue GRE-179, **Define the greenfield ObsCam specification**, and its
accepted child decisions are the architectural system of record. This document
is a concise local projection for agents working in the repository; Linear wins
if the two differ.

## Selected direction

- One continuously warm Rust process exclusively owns the ASI662MC.
- The owner validates the exact SDK model and factory serial before capture.
- Acquisition is full-resolution RAW8 with latest-frame generations and no
  queued stale frames.
- Rust owns neutral monochrome/colour processing, FFmpeg hardware H.264
  orchestration, control, telemetry, exact-correlation evidence, and component
  recovery.
- MediaMTX independently provides direct WHEP/WebRTC fan-out from one shared
  H.264 stream.
- The browser owns interaction, presentation, and snapshot download.
- Runtime state is RAM-only.
- Control uses a server-timed, renewable five-second lease with immediate,
  generation-fenced takeover.
- The deployed-stack acceptance seam exercises browser-facing HTTP, WebSocket,
  control, telemetry, and WHEP contracts through the real Rust service, FFmpeg,
  and MediaMTX wherever the environment permits.

## Rejected paths

Do not introduce or recover these paths unless a later approved Linear decision
explicitly changes the map:

- Python application serving
- JPEG, MJPEG, or any secondary media-delivery path
- server-side recording, image history, or runtime-state persistence
- rotation, automatic binning, scaling, or silent ROI reduction
- native mobile or desktop clients
- application accounts inside the LAN/WireGuard authentication boundary

## Historical evidence

Retained files under `research/` record measurements and conclusions. They are
evidence, not reusable implementation. Deleted source remains available through
Git history only for explicit historical investigation. Never search, restore,
copy, or derive production architecture from it by default.
