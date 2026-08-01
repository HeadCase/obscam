# GRE-221 independent media-recovery verification

## Implemented contract

Rust owns the continuously warm camera, capture settings, control lease, and one
replaceable encoder publication independently. A failed or blocked FFmpeg child
is terminated and replaced without restarting Rust, reopening the camera,
revoking control, or invalidating capture-owned work. The first replacement
attempt is immediate; later failures use jittered exponential backoff capped at
five seconds. Each replacement starts a new correlation stream epoch, clears
old mappings, and records a distinct encoder-replacement counter. Replaced or
discarded pending pipeline work increments a separate skip counter.

The encoder input writer owns the in-flight I420 frame while the supervisor
waits for a bounded response. On timeout, terminating FFmpeg unblocks the writer
and returns that exact owned frame for recovery. Encoder recovery discards only
encoder-pending work; it does not advance the shared capture/settings epoch, so
a long exposure or processing operation already in flight remains valid.

MediaMTX readiness comes from its read-only metrics endpoint on the private
`169.254.218.0/30` veth. The endpoint is not translated to a browser-facing
address. The firewall admits only established or related replies from the
namespace to the host and continues to drop new namespace-to-host traffic.
Relay loss does not stop RTP publication or change camera, encoder, settings,
runtime, or lease ownership.

The browser retains its last trustworthy frame during confirmed media failure.
It reconnects WHEP after authoritative relay or encoder recovery and also
observes peer-connection failure, covering a relay restart too short for the
500 ms relay poll to observe. A new media-connection generation and a new exact
current-stream presentation are required before returning to Live.

## Automated evidence

- Encoder tests cover a child that exits before receiving a frame and a child
  whose stdin blocks until the supervisor timeout kills it and recovers the
  owned frame.
- Backoff tests prove the first retry has zero delay even with maximum jitter
  and repeated failures cap at five seconds.
- Correlation tests prove replacing a stream invalidates all prior-epoch
  mappings.
- Mailbox tests prove encoder-pending work can be discarded without advancing
  the shared capture epoch or rejecting capture work already in flight.
- Runtime tests prove encoder and relay recovery preserve the control lease and
  advance encoder-replacement and pipeline-skip counters independently.
- Browser reducer tests prove failure retains a trustworthy stale frame,
  recovery replaces the dead media connection, and the old connection cannot
  establish Live.
- Deployment tests prove the metrics permission and listener remain private,
  port 9998 is not forwarded, and only established replies can return from the
  owned media veth.

## Browser and deployed-stack evidence -- 2026-08-01

The release Rust service used the real ASI662MC, hardware `h264_v4l2m2m`,
MediaMTX v1.19.3 matching the repository checksum, and the Mac Playwright
Chromium browser over the permitted `http://10.164.190.1:8080` WireGuard route.

- A full MediaMTX and namespace stop/recreate/start left the Rust runtime epoch
  `05da92c8-8d2f-4722-9a16-b7eceb2f55f4` unchanged. Capture and encoder stayed
  Ready, their counters did not advance, RTP publication continued, and the
  private metrics path returned `paths{name="obscam",state="ready"} 1` after
  recovery.
- While MediaMTX was held stopped, the browser showed `Stale · Recovering
  relay`, retained its last trustworthy frame, and continued to report `You
  have control`. After restart, MediaMTX accepted the continuing H.264 path,
  established a fresh WHEP session, and the same browser returned to Live with
  the lease intact.
- A rapid MediaMTX restart also terminated the old WHEP session, created a new
  session from the Mac WireGuard client, and returned to native advancing Live
  video without a Rust restart.
- Sending `SIGKILL` to the exact ObsCam-owned FFmpeg child advanced encoder
  replacements from 2 to 3. Rust PID, runtime epoch, camera readiness, relay
  readiness, settings, and browser lease remained unchanged; a new FFmpeg PID
  published the replacement stream.
- Sending `SIGSTOP` to the exact replacement child during a real 30-second
  exposure exercised the two-second blocked-input timeout. Encoder replacements
  advanced from 3 to 4 while capture retained the exact settings generation,
  exposure, and capture-start timestamp. The browser showed Reconnecting with
  the lease intact, continued retained-frame playback, returned to Capturing,
  and then returned to Live after restoring 500 ms.
- Repeated startup, exit, and hang replacements recovered without restarting
  Rust or reopening the camera. Encoder replacements and pipeline skips remained
  distinct (`4` and `9` after fault injection).

The verification-corpus user-experience regression checks also passed:

- Twelve consecutive half-second samples remained Live; native 1920x1080 video
  advanced 5.749 seconds during the six-second observation.
- Independent gain and exposure changes returned to Live without changing the
  other settings. Isolated monochrome-to-colour and colour-to-monochrome checks
  completed in 2.968 seconds and 5.668 seconds and restored 500 ms, gain 100,
  monochrome.
- A second browser observed disabled controls, explicitly took authority, and
  disabled the first viewer before releasing the lease.
- The controlled hidden-to-visible transition recorded Reconnecting before
  Live and resumed video from 0 to 0.979 seconds on the new connection.
- Mobile 390x844 and desktop 1440x900 viewports had exact document widths, all
  fourteen exposure choices, and reachable status and authority controls.
- A fresh page with listeners attached before navigation reached Live native
  video with no console errors, page errors, or HTTP responses at or above 400.

Fault-injection pages intentionally recorded failed WHEP requests while
MediaMTX was stopped. Those expected diagnostics were excluded from the clean
fresh-page regression check, which attached new listeners before navigation as
required by the browser-testing procedure.
