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
- Production monochrome reconstructs full-resolution neutral luminance from the
  ASI662MC RGGB mosaic using fixed bilinear demosaicing and BT.601 coefficients.
  It uses neutral YUV420 chroma without an intermediate RGB frame or temporal
  image history.
- Rust owns neutral monochrome/colour processing, FFmpeg hardware H.264
  orchestration, control, telemetry, exact-correlation evidence, and component
  recovery.
- MediaMTX independently provides direct WHEP/WebRTC fan-out from one shared
  H.264 stream.
- MediaMTX `v1.19.3` for Linux ARM64 is the deployment pin. Its binary checksum,
  checked-in configuration, and deployed-stack smoke check advance together;
  ambient host versions are not accepted. MoQ remains explicitly disabled.
- ObsCam, MediaMTX, AllSky, and `asiair-sync` are independently supervised
  workloads. ObsCam owns the ASI662MC, ZWO SDK interaction, FFmpeg, browser
  service, control, telemetry, and component recovery; MediaMTX owns WHEP/WebRTC
  fan-out; AllSky remains the exclusive ASI178MC owner. This repository versions
  the independent `asiair-sync` host-integration policy without making it a
  camera-service dependency.
- ObsCam and MediaMTX start from local readiness only and must not wait for each
  other, `network-online.target`, WireGuard, DNS, internet access, or optional
  storage. Expected camera and FFmpeg failures recover inside Rust; unexpected
  process exits receive persistent delayed systemd retries without permanent
  start-limit lockout. A systemd watchdog is not required until evidence shows
  internal recovery cannot detect a hang.
- ObsCam and MediaMTX run as separate unprivileged non-login identities. Camera
  access uses the narrowest permission mechanism proven on the deployed Pi;
  exact model, exactly-one-match, and factory-serial validation remains the
  fail-closed ownership boundary. Hardening is service-specific and retained
  only after ZWO SDK, USB, FFmpeg hardware-encoding, recovery, and diagnostic
  compatibility is verified.
- Shared-host contention uses relative CPU and I/O weights rather than hard CPU
  quotas. WireGuard remains protected infrastructure, interactive ObsCam and
  MediaMTX receive favorable contention weights, AllSky retains normal service
  capacity, and rclone yields first. Memory high/max thresholds are generous and
  evidence-derived; OOM preference selects sync before camera infrastructure.
- `asiair-sync` retains a 512 KiB/s bandwidth cap, one transfer, and two checkers
  with low CPU/I/O priority. A five-minute systemd timer invokes a fail-soft
  oneshot worker which verifies both paths are genuine mounts before copying.
  It has no mount, VPN, network-online, or camera-service dependencies. Missing
  resources defer the cycle for a later retry. Viewer-aware throttling is added
  only if deployed contention tests show static isolation is insufficient.
- Managed services log only through journald with bounded verbosity and per-unit
  rate limits. The deployment provides read-only diagnostics across unit state,
  recent logs, qualified versions, health, mount/VPN state, resources, and
  thermal/throttling signals; health does not create cross-service restart
  coupling.
- A deployment release is one qualified compatibility set: ObsCam and browser
  assets, MediaMTX binary/checksum/configuration, systemd definitions, and the
  expected ZWO SDK ABI/version/checksum. Immutable staged releases advance
  transactionally through a stable active-release link, retain the previous
  known-good release, refuse to overwrite unowned host paths without explicit
  adoption, and automatically roll back when bounded post-activation checks
  fail. Host-specific configuration remains separate and validated.
- Release promotion requires local quality/installer/rollback checks and a
  deployed-Pi gate covering clean install, reboot without remote resources,
  independent crashes, camera identity/disconnect recovery, uninterrupted
  AllSky ownership, VPN and mount failures, concurrent AllSky/rclone/four-viewer
  load, bounded logs and resources, automatic/manual rollback, and the accepted
  GRE-189/GRE-208 field-quality soak. Unavailable required hardware evidence
  blocks promotion rather than being replaced by mocks.
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
- native ASI662MC Y8 and direct RAW8 mosaic-copy monochrome treatments

## Historical evidence

Retained files under `research/` record measurements and conclusions. They are
evidence, not reusable implementation. Deleted source remains available through
Git history only for explicit historical investigation. Never search, restore,
copy, or derive production architecture from it by default.
