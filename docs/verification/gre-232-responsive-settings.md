# GRE-232 Responsive Settings Verification

Verified on 2026-08-05 against the production Raspberry Pi stack, real
ASI662MC, hardware H.264, MediaMTX v1.19.3, and the Mac Playwright browser.

## Qualified camera boundary

- Official ZWO SDK: `libASICamera2.so.1.41`
- SHA-256:
  `3ecf511979ed571131e7d7f4a467112aba21940b4dc91d63bd109b9683bf67c4`
- Capture remained running while exposure and ISO controls changed dynamically.
- The first two frames after a sensor-control change remained visible but were
  withheld from exact correlation as approved transition frames.

## Settings timing

For a 500 ms to 100 ms exposure change:

- Browser draft projection was immediate.
- The UI immediately showed `Requested · awaiting acceptance` after Apply.
- Rust accepted the complete tuple at `15:30:53.937478`.
- Active-exposure abandonment was requested at `15:30:53.937541`.
- The camera applied the tuple at `15:30:53.975553`, 38 ms after acceptance.
- The browser established the first exact Visible frame after 1.463 seconds.

For the return to 500 ms, camera application completed 51 ms after acceptance.
No camera restart occurred for either transition.

The original 500 ms browser-visible acceptance threshold is not met. Honest
transition-frame handling and the current Mac-to-Pi network path remain in that
measurement; transition frames were not relabelled as exact.

## Publication and correlation

- FFmpeg input is paced at 20 fps (one publication every 50 ms).
- Exposures slower than 50 ms repeat the latest completed frame.
- Exposures faster than 50 ms use latest-only replacement and intentionally
  discard intermediate camera frames before encoding.
- Direct FFmpeg PTS and RTP marker pairs remain authoritative when the hardware
  encoder skips an input. Skips no longer force a healthy encoder restart.
- A real-camera soak exceeding seven minutes, including both settings changes,
  completed with zero encoder replacements, zero camera restarts, and zero
  correlation conflicts after direct-pair correlation was deployed.

## Browser diagnostics

- Exposure, ISO, and treatment independently show Draft, Requested, Visible,
  and Rejected evidence without claiming unproved application or visibility.
- A replacement encoder stream fences the prior exact browser presentation.
- Unknown observations remain unknown and are accepted by the diagnostics API;
  stale-stream exact reports no longer produce repeated HTTP 400 responses.
- Clock calibration takes three samples and retains the lowest-round-trip
  result. Concurrent WHEP startup attempts share one quality-client connection.
- Diagnostics initialization runs outside the media startup critical path. An
  end-to-end browser contract blocks the clock endpoint and verifies that the
  first WHEP request still begins without waiting for clock calibration.

The final Mac Playwright probe held clock calibration for 15 seconds. The WHEP
POST began after 7.139 seconds, before calibration was released. The normal
page produced no browser-console errors; the remote route was nevertheless
degraded enough to report `Stale` at 0.7 presented frames per second during the
sample.

The Pi clock endpoint responded locally in 1.2 ms. During this run, the Mac
observed roughly 830-970 ms per clock request over `10.44.0.1` and 688-1,198 ms
over the LAN fallback. Consequently, browser clock uncertainty remained large
and the displayed frame-age number cannot isolate application latency more
precisely than that uncertainty on this network path.

### Diagnostics traffic and startup

The original live diagnostics response retained all 512 raw samples and was
167,223 bytes. Fetching that response as often as every 500 ms created roughly
2.7 Mbps of HTTP traffic per viewer and built TCP send queues on the constrained
WAN path. The live browser now requests a distinct aggregate-only response;
real-browser samples were 1,218-1,234 bytes. The full retained report remains
available only when the operator requests the Quality JSON download.

The production browser code is one minified 48,593-byte bundle instead of an
entry module followed by eight separately discovered modules. Cold Mac browser
contexts began WHEP after 2.942, 5.520, and 6.679 seconds. The remaining spread
tracks the external WAN queue: Pi-to-router ICMP averaged 0.857 ms, while the
same run observed 20-1,205 ms to the Mac WireGuard peer and approximately
1.1 seconds to a public Internet address. WHEP began 162 ms after the runtime
response in the detailed bundled trace.

## Resource pressure

At 500 ms exposure with continuous 20 fps hardware publication:

- ObsCam: approximately 11-15% of one CPU core, 42 MB RSS.
- FFmpeg: approximately 10% of one CPU core, 63 MB RSS.
- Combined usage is approximately 6% of the Pi's four-core CPU capacity.
- RSS was effectively unchanged from the prior variable-rate publication path.

## Multi-viewer acceptance

Four simultaneous Mac browser pages reached Live at native 1920x1080 and all
continued advancing. In the corrected readiness sample, all four advanced over
the same two-second interval. No secondary media path or spatial reduction was
used.
