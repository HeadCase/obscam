# GRE-217 frame-correlation verification

## Implemented contract

Each completed camera generation is stamped immediately after the camera
returns its validated RAW8 buffer. Its settings generation, treatment, and
exposure-completion time remain attached through the latest-only RAW8 and I420
handoffs. Each actual FFmpeg write records the runtime epoch, encoder stream
epoch, source and settings generations, treatment, native dimensions,
exposure completion, submission time, and whether the write is a repeat.

FFmpeg emits H.264 RTP on localhost. Rust forwards the RTP and RTCP packets
unchanged over the private media veth to the pinned MediaMTX RTP source in the
dedicated `obscam-media` network namespace. FFmpeg's direct `muxer <-`
timestamp evidence identifies the exact 90 kHz input-timeline index, while the
RTP marker packet supplies the corresponding actual RTP timestamp. The observer
validates RTP version, payload type, SSRC, and sequence continuity without
blocking the relay. FFmpeg is configured with a fixed initial RTP sequence;
this anchors the lossless ordered PTS and marker streams without waiting for a
periodic RTCP report. The observer requires that first packet and continuous
sequence thereafter, so
missing the first or any subsequent packet permanently invalidates exact
evidence. Every subsequent pair must also agree with the anchored
RTP delta. Queue pressure, packet loss, or disagreement permanently stops
correlation for that stream epoch while media forwarding continues. It never
substitutes frame arrival order or the newest server generation.

If the private-veth route is temporarily unavailable, Rust keeps consuming the
qualified FFmpeg stream and retries each relay send without flooding logs. Media
forwarding resumes when the route returns; correlation remains conservatively
unknown for that stream epoch because missing packets cannot be reconstructed.

The bounded correlator rejects evicted inputs, timestamp resets, fractional or
half-space ambiguous gaps, stale observations, and conflicting timestamps.
Whole 4,500-tick gaps discard only the proven skipped submissions and increment
the encoder-skip counter. At most one completed frame remains held by the
encoder worker and is resubmitted at 20 fps while no newer processed generation
is available; repeats retain the original source identity and receive a fresh
submission time. Settings generations remain on the continuously warm encoder
timeline. The qualified one-second GOP supplies the supported bounded
decodable boundary; stream epochs advance only when the encoder component
itself starts or recovers.

Exact mappings are broadcast through the existing control WebSocket. Each
browser retains at most 128 mappings, clears them on a new stream epoch, and
matches only `requestVideoFrameCallback().rtpTimestamp`. A matching target
settings generation advances that browser's transition from Applied to
Visible. Missing metadata and missing, stale, conflicting, or wrong-epoch
mappings render correlation as Unknown.

## Automated evidence

- Rust correlator tests cover RTP wrap, capacity eviction, whole-step encoder
  skips, repeats, conflicts, fractional gaps, and ambiguous gaps.
- Encoder tests cover direct FFmpeg muxer-PTS parsing, RTP marker parsing, the
  hardware-only profile, and the single RTP output.
- WebSocket serialization tests prove the flat browser mapping contract.
- Browser reducer tests cover exact/missing RTP metadata, capacity-independent
  lookup, stale and conflicting mappings, component epochs, reconnect reset,
  and browser-local Visible advancement.

## Environment-dependent evidence

An isolated deployed-stack check used the deterministic full-resolution camera,
the Pi's real `h264_v4l2m2m`, the pinned MediaMTX v1.19.3 ARM64 binary (matching
repository SHA-256), and Chromium on the operator Mac over WireGuard.

- MediaMTX accepted the checked-in RTP-source shape and reported one H.264
  track; WHEP established over the permitted `10.164.190.1` route.
- The browser decoded native 1920×1080 video and rendered exact Visible source
  generations with no viewport overflow.
- A 500 ms → 20 ms transition remained on the same encoder stream, completed
  browser-local Applied → Visible, and continued advancing video.
- At 5 s exposure, the browser retained one exact source generation while
  bounded repeats kept the shared video session advancing.
- Reload created a new WHEP session and recovered exact Visible correlation.
- The only console error was the pre-existing `/favicon.ico` HTTP 404.

The first deployed attempt restarted FFmpeg at a settings boundary. MediaMTX
correctly rejected the resulting in-place RTP discontinuity as apparent B-frame
reordering and the browser froze. The implementation was corrected to keep the
qualified encoder continuously warm across settings generations. The hardware
encoder's raw-stdin interface does not expose a supported dynamic force-IDR
control, so the qualified one-second GOP is the explicit boundary in that case.
The repeated deployed check passed.

The real ASI662MC exposure-completion edge and Safari remain required production
promotion evidence. Their absence does not invalidate the deterministic
deployed-stack contract, but neither is inferred from it.
