# GRE-213 deterministic camera-substitute verification

## Boundary

`CameraSource` is the capture lifecycle consumed above the hardware boundary.
The production `CameraOwner` and feature-gated `DeterministicCamera` implement
that same contract and return the same borrowed `FrameGeneration` type.

The substitute exists only when `zwo-asi/camera-substitute` is compiled and is
selected only by explicitly constructing a `DeterministicScenario` and passing
it to `DeterministicCamera::connect`. The production service has no substitute
configuration or fallback, so a production build cannot select it silently.

## Deterministic behavior

- Four reusable RAM buffers hold exact 1920×1080 RAW8 RGGB generations.
- Capture waits advance virtual exposure time by at most the production
  100-millisecond wait bound; tests do not sleep or read wall-clock time.
- Scripted plans trigger a one-shot timeout, camera absence, disconnect,
  malformed dimensions, malformed byte length, and additional completion delay.
- Disconnect stops capture. Explicit recovery requires the complete settings
  tuple to be applied again before capture restarts.
- Timeout, interruption, disconnect, and malformed frames never advance the
  trustworthy source generation.
- The generated mosaic contains gain-sensitive RGGB values, asymmetric spatial
  coordinates, a generation-dependent value at every pixel, and a 64-bit
  high-contrast generation barcode in its first two rows. Together these make
  Bayer, crop, rotation, torn/stale-generation, and source-correlation mistakes
  observable.

No encoder, relay, browser-delivery, snapshot, recording, persistence, or
secondary media path is part of the substitute.

## Focused commands

```console
cargo test -p zwo-asi --features 'sdk-stub,camera-substitute' --test deterministic_camera
```

## Completed verification

Passed on 2026-07-30:

- `npm test`
- `npm run build`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-targets --all-features`
- `cargo deny check` (the pre-existing duplicate `syn` versions remain an
  allowed warning)
- `cargo machete`

The full Rust suite was rerun outside the filesystem sandbox because the
existing HTTP-contract tests require a loopback listener; all tests passed.

Mac Playwright verified the real Rust service over the approved WireGuard route
at mobile 390×844 and desktop 1440×900 viewports. Both rendered truthful
`Unavailable` state without overflow. Runtime and health returned HTTP 200,
reported independently unavailable components and `latestFrame: null`. The
only console error was the known pre-existing `/favicon.ico` 404.
