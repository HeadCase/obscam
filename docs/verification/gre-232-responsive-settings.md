# GRE-232 Responsive Settings Verification

Verified on 2026-08-05 against the production Raspberry Pi stack, real
ASI662MC, hardware H.264, MediaMTX v1.19.3, and the Mac Playwright browser.

## Qualified camera boundary

- Official ZWO SDK: `libASICamera2.so.1.41`
- SHA-256:
  `3ecf511979ed571131e7d7f4a467112aba21940b4dc91d63bd109b9683bf67c4`
- Exposure and gain controls change while SDK video acquisition remains warm.
- A longer exposure withholds one transitional frame from exact identity. A
  shorter exposure or gain change withholds at most two. Transitional frames
  remain visible as approved; the camera handle and video acquisition are not
  restarted.

## Accepted real-camera timing contract

Real hardware established that restarting SDK video capture adds a variable
first-frame penalty. The accepted production contract instead keeps capture
warm and uses these browser-visible bounds:

- Normal: requested exposure + 700 ms.
- Hard: requested exposure + 1,000 ms.
- Therefore 5 s to 200 ms is 900 ms normal / 1,200 ms hard.
- Therefore 30 s to 100 ms is 800 ms normal / 1,100 ms hard.

Timing uses the generation-correlated browser mark for the first exact frame,
not button state or a retained prior image.

### Real-camera samples

- 5 s to 200 ms: 824 ms, 623 ms, and 838 ms.
- 30 s to 100 ms: 425 ms, 439 ms, and 550 ms.
- All six samples met the normal target.
- A move from 500 ms to 5 s becomes exact after one requested exposure plus
  processing and delivery, rather than waiting for two 5 s frames.

## Startup and primary status

- Fresh Mac browser samples reached `Live` in 909-1,417 ms.
- Startup no longer waits for an RTCP sender report before declaring advancing
  decoded media Live.
- Primary status progresses through `Unavailable`, `Reconnecting`, and
  `Waiting for first image` to `Live`.
- Healthy retained media remains `Live` during a long exposure and during a
  settings transition. Settings progress remains separately visible.
- A normal source-generation advance does not reconnect WHEP. Runtime-epoch
  changes and genuine component recovery retain their fail-closed reconnects.

## Publication, correlation, and diagnostics

- FFmpeg input remains continuously paced at 20 fps. Slow camera exposures
  repeat the latest completed image; faster camera rates use latest-only
  replacement and may intentionally drop intermediate frames.
- Exact RTP evidence is established directly from FFmpeg PTS and RTP markers;
  correlation no longer waits roughly five seconds for the first RTCP sender
  report.
- A mapping that arrives one browser callback late still proves the most
  recently presented frame while preserving the newer callback as pending.
- Bounded Mac/Pi clock skew no longer rejects an exact RTP identity merely
  because the calibrated browser timestamp is slightly earlier than the Pi
  submission timestamp.
- Service-quality reports are fenced across reconnect generations, preventing
  queued or in-flight observations from a replaced track poisoning the next
  connection.
- The banner reports the current quality partition. Historical startup
  unknowns remain available in the downloadable JSON instead of being mixed
  into current settings diagnostics.
- Final steady samples included `508 exact · 0 unknown` at 100 ms and
  `5 exact · 0 unknown` immediately after the final 5 s to 200 ms transition.

## Edge checks

- Repeated settings changes retained one WHEP quality connection generation;
  settings no longer caused media reconnects.
- Browser-visible media remained Live through the settings series.
- Exact settings partitions contained zero unknown samples.
- No encoder replacement or camera reopen occurred during the final continuous
  capture series.
