# GRE-219 viewer-lifecycle verification

## Implemented contract

The browser has one pure root reducer for authoritative lifecycle facts, media
connection and presentation events, control authority and settings stages,
runtime and stream epochs, correlation, and service-quality evidence. Browser
storage, WebSocket, WHEP, visibility, timers, DOM rendering, and downloads are
effects around that state.

Rust broadcasts an additive lifecycle message on the existing control
WebSocket. It contains the runtime epoch, component facts, recovery component,
and the current authoritative settings generation, exposure, and capture-start
time. The relay's existing `not_observed` value remains unknown rather than
being promoted to readiness or treated as a confirmed failure.

The reducer derives Live, Capturing, Reconnecting, Stale, and Unavailable.
Delivery allowance uses the client-scoped authoritative p99 and is clamped to
250--500 ms. A one-second loss-of-correlation grace retains the last proven
state; afterward generation, age, and latency are omitted. An on-deadline
authoritative exposure remains Capturing even when those frame facts become
unknown. Confirmed capture or encoder recovery immediately produces Stale when
a trustworthy frame exists and Unavailable otherwise. Runtime-epoch changes
retain the prior image only as Stale.

Background tabs suspend tick-based presentation liveness. Foreground return
immediately creates a new media connection generation and WHEP session. Each
video-frame callback is installed by and tagged with that exact generation;
callbacks are cancelled on reconnect and the reducer rejects callbacks from a
different or not-yet-connected session. Control connectivity remains
independent from media state.

The encoder retains its one already-published completed frame across a settings
epoch and repeats that exact identity at 2 Hz until the first new-generation
frame completes. Pending pre-boundary work remains fenced; no frame is relabelled
as the new settings generation.

## Automated evidence

- Browser reducer tests cover every primary state, long exposure and deadline,
  measured allowance bounds, correlation grace, unknown facts, component
  recovery, control/media independence, prior runtime epochs, background and
  foreground behavior, and Accepted/Applied/Visible settings stages.
- Browser boundary tests reject malformed lifecycle facts.
- Browser clock tests prove viewer deadlines use the calibrated Rust service
  clock rather than an uncalibrated browser wall clock.
- Reconnect tests prove cached mappings cannot establish Live in a new media
  connection generation; a current mapping and presentation are required.
- WebSocket contract tests cover initial lifecycle facts and authoritative
  capture-progress updates.
- The FFmpeg contract test proves the owned completed I420 frame remains valid
  for decodability repetition after a settings epoch advances.
- HTTP asset tests prove the root reducer is embedded in the qualified Rust
  binary.

## Browser and deployed-stack evidence -- 2026-07-31

The release Rust service used the real ASI662MC, hardware H.264 encoder, pinned
MediaMTX v1.19.3, and the Mac Playwright Chromium browser over the permitted
`10.164.190.1` WireGuard route.

- Native 1920x1080 video reached Live and displayed the exact source generation
  plus an increasing frame age.
- A 500 ms to 30 s settings transition showed Capturing. After more than 18
  seconds it truthfully showed `Exposure in progress · frame freshness unknown`
  while the retained video time continued advancing through bounded repeats.
- Restoring 500 ms advanced settings generation 2 and returned to Live.
- Foreground return advanced the client media-connection generation from 1 to
  2, exercising the immediate reconnect effect.
- Mobile 390x844 and desktop 1440x900 viewports had no horizontal or vertical
  overflow. The final page had no console errors or warnings.

After review hardening, the same production stack was rerun. The 30 s setting
entered Capturing immediately and remained Capturing with unknown freshness
while the 1920x1080 WHEP video continued playing. Restoring 500 ms completed
the settings transition; the viewer then reported Stale because exact browser
correlation was not refreshed within its grace period, rather than inventing a
Live claim. The 390x844 viewport again had no horizontal overflow, and the
browser reported no console errors or warnings.

Safari, Zen, physical-LAN ingress, four-viewer load, and the production field
soak remain release-promotion evidence owned by the parent specification; they
are not inferred from this focused implementation check.
