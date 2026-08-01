# GRE-220 camera-recovery verification

## Implemented contract

The capture supervisor retries indefinitely after camera absence, identity or
SDK failure, capture failure, an overdue exposure, or repeated malformed source
frames. Camera recovery revokes control authority, discards accepted but
unapplied settings, pauses and drains processing, conclusively stops and closes
the current owner before reconnecting, and restores only the last fully Applied
tuple. A stop or close error terminates Rust rather than risking another
in-process SDK owner. Every new real-camera connection passes through the
existing exact ASI662MC model, factory-serial, native-dimension, RGGB, and RAW8
validation; ASI178MC remains ineligible.

Repeated invalid processing output recovers only processing: it drains and
recreates the processors and fences old work without stopping, reconfiguring,
or reopening the camera. Recovery requests are coalesced until that reset
completes.

Camera-open retries are immediate and then use jittered 1, 2, 4, 8, 15, and
30-second delays, capped at 30 seconds. A dedicated capture watchdog interrupts
an exposure at its configured duration plus two seconds and terminates Rust one
second later if the owner has not completed its stop. The watchdog remains
armed through synchronous SDK teardown, preventing a second SDK owner from
being created while the first may still own capture or its handle.

Invalid dimensions, source buffer lengths, source-generation metadata, and
processing output have independent counters and ten-second failure windows.
The third responsible failure recovers only the responsible component. Source
generations and media epochs remain monotonic across reopened camera owners,
and lifecycle facts publish a minimum acceptable source generation. The browser
may retain an old completed frame during recovery, but that frame cannot clear
Reconnecting or advance settings to Visible; only a correlated presentation at
or above the current minimum can return the viewer to Live.

## Automated evidence

Status: **passed on 2026-08-01**.

- Deterministic camera plans inject SDK errors, disconnects, malformed
  dimensions, malformed lengths, and malformed generation metadata through the
  production `CameraSource` contract.
- Supervisor tests prove absence and initial configuration failure retry until
  capture recovers; the previous owner is closed before reconnecting;
  concurrent owner count never exceeds one; only the Applied settings tuple is
  restored; processing recovery advances its fence without stopping capture;
  and published source generations remain monotonic across owners.
- Subprocess tests prove stop and close failures terminate with status 70 before
  any in-process reopen. Separate monitor tests prove the live watchdog sends
  the owner's interruption request and terminates with status 70 when the armed
  operation does not complete.
- Policy tests prove the complete retry schedule and jitter cap, exact watchdog
  deadlines, per-category sliding failure windows, authority revocation,
  pending-command discard, native I420 processing validation, and one
  edge-triggered processing recovery request per threshold crossing.
- Browser reducer tests prove a retained pre-recovery presentation cannot
  establish Live, including when lifecycle notifications coalesce directly to
  a higher generation floor, and that a new presentation meeting the
  authoritative floor can.
- Runtime, HTTP, and control-WebSocket contract tests prove the additive
  recovery counters and minimum source-generation fence are present in both
  startup and lifecycle facts.

The following repository gates passed:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo deny check
cargo machete
npm test
npm run typecheck:browser
cargo build --release -p obscam
```

`cargo deny check` reported only the repository's allowed duplicate-version
warnings and completed with advisories, bans, licenses, and sources all OK.

## Deployed stack and browser evidence -- 2026-08-01

The installed MediaMTX v1.19.3 binary matched the repository SHA-256, all
installed relay, namespace, firewall, and unit files matched their checked-in
versions, and IPv4 forwarding was enabled. The release Rust service opened the
real ASI662MC and reported capture, encoder, and relay Ready. Runtime facts
started with recovery counters at zero and `minimumSourceGeneration` 1.

Mac Playwright Chromium reached the Pi only through the permitted
`http://10.164.190.1:8080` WireGuard route. The final reviewed release build
reached Live in a fresh 1440x900 page with native 1920x1080 video and an exact
source generation. At 390x844, video advanced from 4.506 to 7.906 seconds,
document width
equalled viewport width, all 19 buttons remained present, and no new console
errors or HTTP responses at or above 400 were observed.

This deployed run also exposed and then verified the fix for a lifecycle
contract omission: the first run rejected lifecycle messages because the
WebSocket omitted `minimumSourceGeneration`; after adding the field and its
regression assertion, the same production path reached Live cleanly.

## Hardware boundary

The exact-identity, factory-serial, dual-camera non-interference, native buffer,
cancellation, and repeated logical reopen evidence remains the deployed
hardware evidence recorded in `gre-212-camera-owner.md`. Physical unplug/replug
and deliberate USB or vendor-SDK fault injection were not performed because
they would disrupt the fixed observatory installation. Those failure paths are
covered by the deterministic substitute and supervisor tests; this record does
not claim a destructive real-hardware fault run.
