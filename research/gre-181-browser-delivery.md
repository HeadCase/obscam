# GRE-181: Low-latency browser delivery architectures

## Question

Which greenfield delivery architecture can carry full-resolution output from the native ZWO SDK to four browser clients with minimum capture-to-display latency, maximum honest frame cadence, materially cheaper B&W operation, and graceful behavior over the observatory LAN and WireGuard VPN?

This investigation treats the existing application as evidence only. Python, its current camera wrapper, MJPEG, and the current server shape are not constraints. The input boundary is a timestamped frame obtained through the native ZWO SDK.

## Executive finding

No paper design can yet select the winner. Two pipelines deserve measurement on the deployed Pi 4:

1. **One hardware H.264 encode, fanned out through WebRTC.** This is the strongest default if 30 unique displayed frames per second satisfies the eventual service target. It is the only candidate here that combines a Pi 4 hardware codec, broadly interoperable browser video, loss-aware real-time transport, standardized telemetry, straightforward `<video>` rendering, and credible fullscreen/PiP support.
2. **One independently framed JPEG encode, fanned out as timestamped binary frames.** WebSocket is the compatibility baseline; WebTransport is a higher-complexity experiment for discarding obsolete data under loss. This path could exceed the published H.264 ceiling and should make B&W materially cheaper, but its actual Pi JPEG throughput, four-client bitrate, VPN behavior, and browser decode/presentation ceiling are unknown. Rendering is normally to a canvas, so standard PiP is lost.

MJPEG-over-HTTP should remain a control measurement, not an assumed design. MSE/fMP4 is a fallback comparator for H.264, not the leading real-time transport. HLS and LL-HLS are incompatible with the latency priority and should not advance.

The decisive benchmark is: at full 1920x1080 resolution, under co-resident production load and with four viewers, which finalist minimizes p95 frame age while maximizing unique presented frames, without allowing any slow client to reduce camera acquisition cadence or age the other clients' feeds?

## Binding context

- Raspberry Pi 4 Model B, 4 GB, is the binding host and is shared with WireGuard, `rclone`, Allsky, and another ZWO camera.
- ASI662MC full-frame output is binding. There is no automatic binning, scaling, or ROI degradation; only compression may degrade.
- B&W is the default operational mode; colour is on demand.
- Capture changes interrupt the current exposure and restart immediately.
- Four concurrent viewers are required; an iPhone is the primary interactive client, with iPad/macOS Safari and Chromium desktop also relevant.
- Capture-to-display latency takes priority over cadence. Queues must not accumulate: obsolete work is discarded and the newest decodable frame wins.
- Access is LAN-only, extended remotely through WireGuard. Separate application accounts are out of scope.
- Fullscreen and PiP are desirable but must not dictate a slower or less reliable transport. PWA installation is optional.

## Primary-source facts

### Pi 4 encode boundary

- Raspberry Pi publishes **H.264 1080p30 encode** as the Pi 4 capability; it does not publish a higher guaranteed H.264 encode rate. The same specification lists Gigabit Ethernet. [[Raspberry Pi 4 specifications](https://www.raspberrypi.com/products/raspberry-pi-4-model-b/specifications/)]
- Raspberry Pi's current camera documentation says the libav backend uses hardware H.264 when available. Earlier Pi encoders do not generate B-frames; the Pi 5-specific low-latency option suppresses B-frames only on Pi 5 and later. [[Raspberry Pi camera software](https://www.raspberrypi.com/documentation/computers/camera_software.html#libav-integration-with-rpicam-vid)]
- The same documentation exposes target bitrate, I-frame frequency, H.264 baseline/main/high profiles, and an `inline` option that repeats sequence headers at each I-frame for streaming. [[Raspberry Pi video options](https://www.raspberrypi.com/documentation/computers/camera_software.html#video-options)]
- The Raspberry Pi kernel's `bcm2835-codec` source lists H.264, JPEG, and MJPEG compressed formats and raw formats including YUV420, RGB24/BGR24, and 8-bit GREY. It allocates up to 4 MiB for a JPEG output buffer versus 768 KiB above 720p for other compressed frames. This proves the driver has relevant format plumbing, **not** that every format is accepted for the encoder role on the deployed firmware/kernel or what rate it sustains. [[Raspberry Pi `bcm2835-v4l2-codec.c`](https://github.com/raspberrypi/linux/blob/rpi-6.6.y/drivers/staging/vc04_services/bcm2835-codec/bcm2835-v4l2-codec.c)]
- The Linux stateful encoder contract is queue based: raw frames enter the OUTPUT queue and encoded frames leave CAPTURE. Runtime format/control enumeration is therefore part of deployment discovery, not a compile-time assumption. [[Linux V4L2 stateful encoder API](https://docs.kernel.org/userspace-api/media/v4l/dev-encoder.html)]

**Inference:** 1080p30 is a serious architectural ceiling for hardware-H.264 delivery, not a safe prediction of actual ObsCam performance. If the sensor and browser can present materially more than 30 unique frames per second at short exposures, H.264 may leave useful performance unused. Conversely, JPEG may hit CPU, memory-copy, USB, or network limits first. Both require measurement.

### Compression is mandatory

One uncompressed 1920x1080 8-bit B&W frame is 2,073,600 bytes. At 60 fps that is about 995 Mbit/s before framing for **one** viewer; four copies are about 3.98 Gbit/s. RGB24 is three times larger. These arithmetic lower bounds already exceed the Pi's published Gigabit Ethernet capacity, so an uncompressed browser transport is not credible.

### B&W is pipeline-dependent, not automatically faster

- WebRTC browsers must implement VP8 and H.264 Constrained Baseline. In the absence of contrary signalling, WebRTC video uses Y'CbCr 4:2:0. [[RFC 7742](https://www.rfc-editor.org/rfc/rfc7742)]
- The Pi codec driver lists both 8-bit GREY and YUV420 raw formats, but supported formats are populated per codec role at runtime. [[Raspberry Pi codec driver](https://github.com/raspberrypi/linux/blob/rpi-6.6.y/drivers/staging/vc04_services/bcm2835-codec/bcm2835-v4l2-codec.c)]

**Inference:** A B&W H.264 picture can have essentially constant chroma and should need fewer encoded bits at equal quality, but it may still traverse a 4:2:0 input/encoder path and remains subject to the same published 1080p30 hardware ceiling. It must not be promised a higher frame rate without evidence. B&W's CPU gain may instead come from avoiding debayer and colour processing.

For independent JPEG frames, a direct 8-bit luma path can avoid colour conversion and naturally produces less source data than RGB. That makes a significant B&W gain plausible, especially at fixed JPEG quality, but the deployed V4L2 JPEG encoder's GREY acceptance, encode latency, output size, and concurrency must be measured.

### WebRTC provides the strongest standardized real-time behavior

- WebRTC is a W3C Recommendation for sending real-time media and data to browsers. The application supplies signalling; WebRTC supplies ICE connectivity, DTLS/SRTP media security, RTP/RTCP, receiver feedback, and statistics. [[WebRTC Recommendation](https://www.w3.org/TR/webrtc/)]
- Browsers and non-browser WebRTC video endpoints must implement VP8 and H.264 Constrained Baseline, including H.264 RTP packetization mode 1 and in-band sequence/picture information. [[RFC 7742](https://www.rfc-editor.org/rfc/rfc7742)]
- When ICE fails, the Recommendation advises an ICE restart; applications may also use connection state and stats to decide whether to restart after a disconnect. [[WebRTC connection recovery](https://www.w3.org/TR/webrtc/#dom-rtciceconnectionstate-failed)]
- `RTCRtpReceiver.jitterBufferTarget` lets an application request a delay/recovery tradeoff, but the browser clamps the actual target to what it can provide. The actual average can be derived from jitter-buffer statistics. [[WebRTC `jitterBufferTarget`](https://www.w3.org/TR/webrtc/#dom-rtcrtpreceiver-jitterbuffertarget)]
- `getStats()` exposes synchronized transport/media observations, including loss, bytes, frames, and jitter-buffer measurements where implemented. [[WebRTC Stats](https://w3c.github.io/webrtc-stats/)]
- `HTMLVideoElement.requestVideoFrameCallback()` is available across current browser families and reports presentation metadata. For WebRTC it can include capture, network receive, decode, and expected-display times; unique `presentedFrames` changes expose missed presentation callbacks. [[MDN video-frame callback](https://developer.mozilla.org/en-US/docs/Web/API/HTMLVideoElement/requestVideoFrameCallback)] [[HTML video-frame callback specification](https://html.spec.whatwg.org/multipage/media.html#dom-video-requestvideoframecallback)]

**Inference:** WebRTC is the lowest-risk way to avoid application-level media queues under VPN jitter. It does not make "latest frame wins" literal: H.264 frames depend on earlier frames, browsers maintain jitter buffers, and retransmission/congestion policy is browser controlled. A short time-bounded GOP and prompt keyframe handling are necessary to bound recovery and new-viewer join time.

### Custom framed delivery gives stronger application control but inherits transport costs

- WebSocket is available in current browser engines. Its API reports `bufferedAmount`, but calls to `send()` queue reliable ordered bytes; already queued messages are not discarded when closing starts. [[WebSockets Standard](https://websockets.spec.whatwg.org/#the-websocket-interface)]
- The July 2026 WebTransport Working Draft provides reliable streams and datagrams over HTTPS and exposes datagram age/buffer limits. Incoming datagrams beyond the configured queue are dropped from the head; outgoing queues exert backpressure; datagram size is path/protocol limited. The document is still a work in progress despite browser implementations. [[WebTransport Working Draft](https://www.w3.org/TR/webtransport/#webtransportdatagramduplexstream-interface)]
- WebTransport's low-latency congestion-control request is only a hint and is explicitly marked at risk because browsers may not have a matching algorithm. It requires an `https` URL. [[WebTransport connection options](https://www.w3.org/TR/webtransport/#webtransportoptions-dictionary)]
- MDN's browser-compatibility data reports WebCodecs `VideoDecoder` in Chromium from 94 and Safari/iOS from 16.4, while codec availability still requires `isConfigSupported()` probing. [[MDN browser compatibility data](https://github.com/mdn/browser-compat-data/blob/main/api/VideoDecoder.json)] [[WebCodecs](https://www.w3.org/TR/webcodecs/)]
- WebCodecs exposes low-level asynchronous decode queues and reset/close operations, but does not provide container demuxing or a media transport. [[WebCodecs processing model](https://www.w3.org/TR/webcodecs/#processing-model)]
- MDN marks WebTransport as newly Baseline across latest browser versions in March 2026; older production devices remain a compatibility risk. [[MDN WebTransport](https://developer.mozilla.org/en-US/docs/Web/API/WebTransport)]

**Inference:** A WebSocket/JPEG client can keep at most one decode in flight and one replaceable pending frame, but TCP loss can still hold a newer frame behind old bytes already in the network stack. WebTransport datagrams can expire obsolete fragments and avoid reliable-stream head-of-line blocking, but full JPEG frames exceed a datagram and therefore require application fragmentation, reassembly, loss accounting, and whole-frame discard. QUIC/TLS CPU and operational complexity on the shared Pi are unmeasured.

### MSE and HLS favor continuous playback over freshest-frame semantics

- Media Source Extensions append initialization/media segments into browser-managed `SourceBuffer` queues, track buffered/seekable ranges, and can reject appends when resources are exhausted. [[Media Source Extensions](https://www.w3.org/TR/media-source-2/)]
- The ISO BMFF MSE byte-stream format requires initialization and media segments; an H.264 implementation therefore needs fMP4 muxing, keyframe-aware fragments, buffer eviction, and live-edge management. [[MSE ISO BMFF byte stream](https://www.w3.org/TR/mse-byte-stream-format-isobmff/)]
- HLS is playlist/segment based and was designed for reliable adaptive delivery and caching to large audiences. [[RFC 8216](https://www.rfc-editor.org/rfc/rfc8216)]
- The May 2026 HLS second-edition draft recommends a six-second target duration and a one-to-two-second GOP for its low-latency server profile. It requires clients to consider hold-back and explicitly trades shorter targets against stalls and overhead. [[HLS second-edition draft](https://datatracker.ietf.org/doc/html/draft-pantos-hls-rfc8216bis-22#appendix-B.1)]

**Inference:** MSE/fMP4 can be tuned far below HLS latency, but it still uses a reliable byte stream and an explicit browser media buffer. It duplicates work WebRTC already standardizes and makes bounded frame age an application responsibility. HLS/LL-HLS is decisively aimed at a different latency/reliability point than telescope-slew monitoring.

### Fullscreen, PiP, and PWA implications

- Standard PiP is requested on an `HTMLVideoElement`, requires transient user activation, is not guaranteed, and remains limited across browsers/platforms. [[Picture-in-Picture](https://www.w3.org/TR/picture-in-picture/)]
- Fullscreen is also permission/user-activation controlled and must be feature detected. [[Fullscreen Standard](https://fullscreen.spec.whatwg.org/)]

**Inference:** WebRTC, MSE, and HLS naturally render to `<video>` and are best positioned for PiP. JPEG rendered into `<canvas>` can still fill the viewport and may enter fullscreen, but it has no standard PiP path. Mirroring a canvas into a generated media track would add compatibility, buffering, and latency risk solely for a nice-to-have feature, so it should not affect the transport decision.

PWA installation changes launch/display chrome, not the media transport. The service worker should cache the static application shell only, never a live stream or frame response. Foreground/background suspension on iOS must be tested; an installed PWA must not be assumed to keep decoding indefinitely in the background.

## Candidate comparison

| Candidate | Latency and loss behavior | Pi 4 cost | Four-viewer fan-out | Browser/UI fit | Disposition |
|---|---|---|---|---|---|
| **WebRTC + H.264** | RTP/RTCP, loss feedback, per-peer congestion/jitter handling; short GOP bounds loss/join recovery; browser buffering is influenceable but not fully controlled | Published hardware ceiling 1080p30; raw-to-encoder conversion and encode latency unknown | Encode once; packetize/encrypt/pacing per peer. A slow peer must not retune the shared encode or block others | Standards-based `<video>`; best telemetry, fullscreen, and PiP path; H.264 mandatory for WebRTC browsers | **Finalist and recommended default if measured service targets fit at <=30 fps** |
| **JPEG frames over WebSocket** | Every frame independently decodable and app queues can replace pending frames; TCP loss/HOL can age data already in flight | Hardware JPEG path exists in driver source; GREY/RGB rates and copy costs unknown; bitrate much higher than interframe video | Encode once, share immutable buffer, one bounded sender per client; network bytes multiply by four | Broad compatibility; canvas gives exact frame IDs/timestamps; no standard PiP | **Finalist and compatibility baseline for high-cadence/B&W path** |
| **JPEG fragments over WebTransport datagrams** | Old fragments can expire and incomplete frames can be discarded, matching freshest-frame semantics; application must fragment/reassemble and request recovery | Same JPEG cost plus QUIC/TLS, fragmentation, and server complexity | Per-client datagram queues isolate lag; network bytes still multiply by four | Current latest browsers only; HTTPS/HTTP3 required; canvas/no standard PiP | **Benchmark experiment only; advance over WebSocket only if VPN-loss results materially improve** |
| **HTTP multipart MJPEG** | Independent frames and immediate decoder recovery, but reliable response bytes can queue; limited application visibility/control of browser buffering | Similar JPEG encode and bitrate; minimal application protocol | One response/socket per client; bounded server queues possible but kernel/browser buffering remains | De-facto `<img>` behavior must be device tested; not a video element; weak presentation telemetry | **Control baseline, not assumed production architecture** |
| **MSE + H.264/fMP4 over HTTP or WebSocket** | Tiny fragments and live-edge seeking can reduce delay; reliable ordered bytes and browser `SourceBuffer` can accumulate old media | Same 1080p30 H.264 ceiling plus muxing | Encode/mux once and fan out; client buffers isolated | `<video>`, fullscreen/PiP; broad but implementation-specific low-buffer tuning | **Fallback comparator if WebRTC integration, not performance, proves unacceptable** |
| **H.264 over WebTransport + WebCodecs** | Full application control, but must recreate packet loss, keyframe, pacing, decode-queue, and recovery policy | Same H.264 ceiling plus QUIC and custom protocol | Encode once; per-client state | Recent APIs, canvas rendering, no standard PiP without another bridge | **Do not advance initially; no clear benefit over WebRTC** |
| **HLS / LL-HLS** | Segment/partial-segment hold-back and playback buffers prioritize continuity; expected delay is far above frame-level interactive paths | H.264 encode plus playlist/segment machinery | Excellent scalable HTTP fan-out, unnecessary for four viewers | Excellent Safari/video/PiP compatibility | **Reject for the primary live view** |
| **Raw RGB/GREY** | Simple and independently framed but saturates network before four clients and moves decode/convert work to every browser | Extreme memory/network bandwidth | Four copies are infeasible on Gigabit | Canvas only | **Reject** |

## Architecture properties common to either finalist

These are design requirements, not implementation prescriptions:

1. **Single acquisition and encode.** The native ZWO SDK produces each frame once. The selected compressor processes it once. Fan-out references one immutable encoded buffer; client count must not duplicate capture, debayer, or encode work.
2. **No unbounded queues.** Each stage exposes counts and age. Acquisition owns at most the current raw frame plus a replaceable next frame. Each client owns a small bounded egress state. A lagging client loses frames or reconnects; it never slows acquisition or other clients.
3. **Independent client lifecycle.** Joining, leaving, rotating, entering background, or losing VPN on one device cannot restart capture or the shared encoder.
4. **Time-bounded decodability.** JPEG satisfies this per frame. H.264 needs repeated in-band SPS/PPS, a short GOP expressed in elapsed time rather than a fixed frame count, and a way to request/force an IDR after join, mode change, or loss.
5. **Truthful timestamps and frame IDs.** Every captured frame has a monotonic ID and timestamps for exposure end, SDK availability, encode completion, and fan-out. The browser records receive, decode, and presentation observations. Displayed FPS counts unique presented IDs, never request iterations or duplicated frames.
6. **Mode changes are discontinuities.** Colour/B&W or exposure changes abort current capture as already decided, reset obsolete processing state, and make the first new output independently decodable. Old-mode frames are never displayed after acknowledgement.
7. **Per-client backpressure isolation.** H.264 sender feedback may pace/drop for that peer but must not reduce the shared source resolution or camera cadence. With one shared encode, dynamic bitrate changes that affect every viewer require an explicit global policy rather than silent reaction to the weakest client.
8. **Stateless reconnection.** Browsers reconnect with exponential backoff and jitter. No persistent viewing session or server-side master record is needed. Reconnection obtains current settings and the newest independently decodable output.

## Required benchmark before architecture selection

### Runtime discovery

On the deployed kernel/firmware, enumerate the relevant V4L2 encoder nodes, OUTPUT/CAPTURE formats, profiles, bitrate modes, GOP controls, and buffer requirements. Specifically prove or disprove:

- H.264 accepts the native-adapter output without a CPU-heavy RGB-to-YUV copy.
- The encoder accepts GREY directly; if not, measure GREY-to-YUV420 neutral-chroma conversion.
- JPEG/MJPEG accepts GREY and RGB/BGR directly.
- DMA-backed or otherwise zero/low-copy handoff from the native SDK buffer is feasible. Treat it as an optimization to prove, not a prerequisite.

### Pipelines to build as throwaway probes

1. Native SDK -> minimal conversion -> V4L2 H.264 -> one local sink.
2. The same H.264 output -> one and four WebRTC peers.
3. Native SDK -> V4L2 JPEG (GREY and colour) -> one local sink.
4. The same JPEG output -> one and four WebSocket clients.
5. WebTransport datagram-fragment delivery only after the WebSocket probe establishes JPEG encode/decode viability.
6. Existing-style multipart MJPEG and MSE/fMP4 as controls, not production implementations.

### Test matrix

- Full 1920x1080 only; no scaling, binning, or ROI.
- B&W and colour.
- Representative shutters: 10 ms, 20 ms, 50 ms, 100 ms, 500 ms, 1 s, and a multi-second dark exposure.
- One viewer and four simultaneous viewers.
- Current Safari on the actual iPhone (primary), iPad, and Mac; current desktop Chromium. Test normal tab and installed-PWA launch where supported.
- Direct LAN, normal WireGuard path, and controlled bandwidth/latency/loss profiles. Include VPN interruption, interface change, and reconnection.
- Idle host and realistic co-resident load from Allsky, both WireGuard roles, and `rclone`; include sustained thermal behavior.
- Static dark scene, typical observatory scene, and high-motion slew. Compression and bitrate conclusions from one scene are invalid.

### Measurements

- Camera acquisition cadence and stale/aborted frame counts.
- Conversion, encode, packetization, encryption, and per-client fan-out latency: p50/p95/p99 and maxima.
- Client receive-to-decode and decode-to-present latency; unique presented FPS and dropped/duplicated IDs.
- End-to-end exposure-end-to-visible latency, p50/p95/p99, with a synchronized instrumentation path and an independent visual high-speed-camera cross-check if available.
- Encoded bytes/frame and Mbit/s per client and aggregate, at equivalent operational image usefulness.
- CPU by process/thread, memory and copy volume, kernel socket backlog, WireGuard cost, temperature, frequency/throttling, and effects on Allsky/VPN reliability.
- Join-to-first-live-frame time, loss-to-current-frame recovery time, and maximum frame age during congestion.
- B&W gain reported separately as acquisition, conversion, encode, bitrate, decode, and presentation deltas. "Significantly more performant" needs a numeric acceptance threshold in the performance-requirements ticket.

For WebRTC, record ICE state, RTT, packet loss, NACK/PLI/FIR, frames dropped, jitter-buffer delay, decode time, and `requestVideoFrameCallback` metadata. Test the minimum requested `jitterBufferTarget` actually honored by each browser.

For JPEG/WebSocket, constrain user-space pending data to one in-flight frame plus one replaceable latest frame; cap socket buffers; record `bufferedAmount`/server pending bytes; and close/reconnect a client whose byte age exceeds the liveness threshold. For WebTransport, record fragment loss, incomplete-frame discard, expiration, QUIC RTT, and CPU.

## Decision rule

1. Choose **WebRTC/H.264** if it meets the quantitative latency/cadence requirements under four-viewer production load. Its 30 fps published ceiling is acceptable only if the requirements ticket makes that outcome acceptable based on observation, not convenience.
2. If WebRTC misses required cadence because of the H.264 ceiling, choose the **independent JPEG family** only if the JPEG probe shows a higher unique presented cadence at bounded frame age and sustainable four-client bitrate. Select WebSocket unless WebTransport materially improves degraded-VPN frame age/recovery enough to justify recent-client and HTTP/3 complexity.
3. If neither meets the requirements at full resolution, document the measured bottleneck and reopen service targets or the exceptional hardware-upgrade path. Do not silently introduce spatial degradation.

## Uncertainties

- Actual full-frame ASI662MC output formats/cadence and the cheapest correct B&W derivation are owned by the capture research, not established here.
- The deployed Pi firmware/kernel may expose a different subset or behavior from the cited Raspberry Pi kernel branch.
- Pi 4 JPEG hardware throughput and latency are not published as a stable product capability.
- The Pi 4 H.264 encoder may exceed or fail to reach 1080p30 in a particular configuration; only the published guarantee is known.
- One shared H.264 encode cannot independently lower bitrate for a weak viewer without affecting all viewers; the operational bandwidth envelope is unknown.
- Browser minimum versions in the actual device fleet are unknown. WebTransport is especially recent; WebCodecs codec/profile support must be probed, not inferred from API presence.
- Safari/iOS fullscreen, PiP, foreground/background, and installed-PWA behavior vary by OS version and policy and require tests on the user's devices.
- Observatory uplink capacity and loss characteristics for four simultaneous VPN streams are not yet quantified.
- Whether WireGuard interface changes preserve a WebRTC candidate pair or require ICE restart is environment-specific.

## Decision implications

- The delivery decision must stay blocked on full-pipeline benchmarks, not just camera-SDK throughput.
- The benchmark milestone should explicitly compare H.264/WebRTC against independent JPEG/WebSocket; otherwise the design cannot know whether it traded away short-exposure cadence for codec efficiency.
- B&W's promised performance advantage must be specified as measured stage-level and end-to-end deltas. A lower bitrate alone is not evidence of higher displayed cadence.
- Fullscreen is transport-neutral. PiP favors a `<video>` path but remains a secondary criterion.
- No research supports HLS/LL-HLS as a primary monitoring transport for this latency-first product.
- A new native camera adapter can expose precise buffers/timestamps and avoid wrapper copies, but it does not remove the Pi encode/network/browser limits identified here.
