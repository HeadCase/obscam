# GRE-232 Responsive Settings Verification

Verified on 2026-08-05 against the production Raspberry Pi stack, real
ASI662MC, hardware H.264, MediaMTX v1.19.3, and the Mac Playwright browser.

## Qualified camera boundary

- Official ZWO SDK: `libASICamera2.so.1.41`
- SHA-256:
  `3ecf511979ed571131e7d7f4a467112aba21940b4dc91d63bd109b9683bf67c4`
- Exposure and gain controls change while SDK video acquisition remains warm.
- Full-resolution RAW processing consumes only the newest available source
  frame. Camera acquisition remains continuous, bounded latest-frame mailboxes
  prevent backlog, and the encoder alone caps browser publication at 20 fps.
- The production exposure floor is 50 ms, matching the ASI662MC's 20 fps sensor
  ceiling. It avoids the qualified 20 ms fast-to-long mode-entry penalty and
  removes the need for an internal bridge setting.
- A longer exposure withholds one transitional frame from exact identity. A
  shorter exposure or gain change withholds at most two. Transitional frames
  remain visible as approved; the camera handle and video acquisition are not
  restarted.

## Accepted exposure-shortening timing contract

Real hardware established that restarting SDK video capture adds a variable
first-frame penalty. The accepted production contract instead keeps capture
warm and uses these browser-visible bounds when moving to a shorter exposure:

- Normal: requested exposure + 700 ms.
- Hard: requested exposure + 1,000 ms.
- Therefore 5 s to 200 ms is 900 ms normal / 1,200 ms hard.
- Therefore 30 s to 100 ms is 800 ms normal / 1,100 ms hard.

Timing uses the generation-correlated browser mark for the first exact frame,
not button state or a retained prior image.

Entering a longer exposure is deliberately outside this shortening SLA. An
initial mode-entry lag is accepted; once visible, captured-frame cadence must
settle to the requested exposure. The qualified 50 ms to long-exposure samples
became exact within requested exposure plus 1.9 seconds.

### Real-camera samples

- 5 s to 200 ms: 824 ms, 623 ms, and 838 ms.
- 30 s to 100 ms: 425 ms, 439 ms, and 550 ms.
- All six samples met the normal target.
- A move from 500 ms to 5 s becomes exact after one requested exposure plus
  processing and delivery, rather than waiting for two 5 s frames.

## Startup and primary status

- Fresh Mac browser samples reached `Live` in 909-1,417 ms.
- Startup does not wait for an RTCP sender report before declaring advancing
  decoded media Live. Exact correlation is anchored independently by FFmpeg's
  fixed initial RTP sequence and fails closed on the first or any later missing
  packet, so media availability never weakens correlation truthfulness.
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

- During investigation, an experimental RAW-processing pacer reduced redundant
  work. Before it, a 20 ms floor acceptance run measured roughly
  140-150% aggregate CPU while processing approximately 50 full-resolution
  source frames per second for a 20 fps encoder. Latest-only 20 fps processing
  reduced the observed aggregate to roughly 85-90%, reduced the processing
  thread from roughly 60% to roughly 16%, and retained browser-visible 19.9 fps
  with 0 unknown samples. The pacer was subsequently removed: at the qualified
  50 ms floor the sensor itself supplies at most 20 source frames per second,
  while the existing latest-frame mailboxes retain the required boundedness.
- Despite that lower load, an immediate 20 ms to 1 s transition still delivered
  no frame before the existing four-second watchdog terminated the process.
  CPU contention was therefore ruled out as the cause of that transition
  failure. The investigated 20 ms floor removed the separate <=18 ms SDK
  mode-entry stall, and the experimental processing pacer removed redundant
  work, but neither qualified fast-to-long full-pipeline transitions.

### Fresh-owner 20 ms mode-entry investigation — 2026-08-06

A tagged diagnostic build retained continuously warm acquisition, recorded each
bounded SDK wait, and temporarily moved watchdog cancellation from exposure plus
2 seconds to exposure plus 10 seconds. The diagnostic margin remained finite
and was restored after the measurements; all tagged probes were then removed.

The minimal deployed-stack reproduction is a newly opened camera owner, one
successful 20 ms frame, and its first request for a long exposure. Gain, repeated
settings changes, and a long dwell at 20 ms are not required. Direct 500 ms to
2 s operation on a fresh owner remained healthy.

| Fresh-owner transition | First SDK result | Browser exact | Outcome |
| --- | ---: | ---: | --- |
| 20 ms -> 500 ms | transitional frame after 4 ms | 814 ms | healthy |
| 20 ms -> 1 s | frame after 1.034 s in a healthy run | 2.333 s | boundary is intermittent; another run recovered after watchdog cancellation |
| 20 ms -> 2 s, production cancellation | 160 timeouts over 4.025 s | none before cancellation | camera recovery; production process terminated during teardown |
| 20 ms -> 2 s, deferred cancellation | frame after 6.070 s | 8.377 s | natural recovery without restart |
| 20 ms -> 5 s, production cancellation | 279 timeouts over 7.018 s | 16.447 s after recovery | camera recovery |
| 20 ms -> 5 s, deferred cancellation | frame after 9.077 s | 14.934 s | natural recovery without restart |
| 50 ms -> 2 s | transitional frame after 1.064 s | 3.341 s | healthy |
| 20 ms -> 50 ms -> 2 s bridge, run 1 | 50 ms exact after 329 ms | 3.856 s after bridge | healthy under production watchdog |
| 20 ms -> 50 ms -> 2 s bridge, run 2 | 50 ms exact after 319 ms | 3.844 s after bridge | healthy under production watchdog |

The SDK did not block inside `ASIGetVideoData`: every observed wait remained at
or below approximately 25-26 ms. It returned only timeout for the requested
exposure plus approximately 4.07 seconds, then produced a valid frame without a
drop-counter increase. The penalty was fixed across 2 s and 5 s targets rather
than proportional to exposure. After one successful long-mode acquisition, a
subsequent 20 ms to 2 s transition on the same logical session produced its
first frame in 2.036 s and browser exact visibility in 4.344 s.

The production watchdog's message also overstated the failure. Capture observed
its cancellation at the next bounded SDK return, but SDK stop/reopen took about
1.34-1.40 seconds while the watchdog allowed only one second between cancellation
and process termination. Production now retains cancellation at requested
exposure plus two seconds but allows three seconds for teardown before process
termination.

These results rule out CPU contention, gain, source dwell, SDK-call
uninterruptibility, browser correlation, and a growing frame backlog. They
identify two separate policies for a production decision: a one-time,
transition-aware allowance for the first 20 ms to long-exposure mode entry, and
a teardown deadline that reflects measured SDK stop/reopen time.

The explicit 50 ms bridge checks provide a faster option than widening the
mode-entry watchdog. On two independent fresh owners, one completed 50 ms frame
between 20 ms and 2 s avoided the SDK penalty and retained warm acquisition.
The bridge itself became exact in 319-329 ms, and the requested 2 s setting
became exact 3.844-3.856 s later. A production bridge would need to keep this
intermediate sensor state internal and mark its frame untrusted; it must not
misreport the bridge as either the prior 20 ms setting or the requested long
setting.

### Production 50 ms floor qualification — 2026-08-06

The restored production binary, production watchdog margins, real ASI662MC,
MediaMTX, and Mac Playwright browser were used without diagnostic
instrumentation. Every exposure sample started with a newly opened camera owner.

| Transition | ISO | Browser exact | Result |
| --- | ---: | ---: | --- |
| 50 ms -> 1 s | 0 | 2.334 s | Live, 0 unknown, no recovery |
| 50 ms -> 1 s | 100 | 2.828 s | Live, 0 unknown, no recovery |
| 50 ms -> 2 s | 0 | 3.854 s | Live, 0 unknown, no recovery |
| 50 ms -> 2 s | 100 | 3.854 s | Live, 0 unknown, no recovery |
| 50 ms -> 5 s | 0 | 6.881 s | Live, 0 unknown, no recovery |
| 50 ms -> 5 s | 100 | 6.877 s | Live, 0 unknown, no recovery |

Six navigation-to-Live samples ranged from 913 ms to 1.439 s. Repeated 5 s to
50 ms transitions became exact in 846 ms, 830 ms, and 824 ms. Two intervening
returns to 5 s became exact in 5.343 s and 5.353 s. A rapid sequence remained
on one live process with 0 unknown samples: 100 ms in 332 ms, 300 ms in 829 ms,
1 s in 1.329 s, 200 ms in 1.325 s, 2 s in 2.350 s, and 50 ms in 818 ms.

After a 15-second steady sample at 50 ms, the browser reported 512 exact, 0
unknown, 19.9 fps, 157 ms p95 presentation latency, and 138 ms frame age. The
qualification build with the experimental processing pacer used 43.6 MiB
resident memory and approximately 55-75% aggregate CPU. After removing that
pacer, the final release used 40.2 MiB and 40-63% aggregate CPU across a
ten-second sample at 50 ms and ISO 0. The same pacer at 20 ms had measured
approximately 85-90% aggregate CPU.

ISO 600 initially exposed an independent correlation blocker. At 50 ms, the
high-noise encoded burst overran the local UDP/5002 receive socket: direct RTP
evidence showed a sequence jump from 8150 to 8157, while the kernel reported 131
dropped datagrams. A 1 MiB per-socket receive buffer remained insufficient in a
stress run. The production RTP observer now requests the host's existing 4 MiB
per-socket ceiling without changing system settings. Three subsequent 50 ms ISO
0 -> 600 -> 0 cycles made all six targets exact in 384-1,046 ms, stayed Live,
emitted no correlation warning, and ended with zero kernel socket drops. The
observer also records the precise invalidation cause if evidence is lost again.

A final 50 ms to 2 s browser transition became exact in 3.347 s. During the
following long-exposure sample the diagnostics reported 0.5 captured fps with
241 exact and 0 unknown samples, confirming steady capture cadence matches the
requested two-second exposure. Returning to 50 ms became exact in 497 ms.

The 50 ms exposure floor therefore avoids every reproduced 20 ms SDK mode-entry
failure, retains the 20 fps browser ceiling with lower resource pressure, and
needs no internal bridge transition. Fresh 50 ms to 5 s samples were consistent
with the separately qualified longer-exposure entry envelope above rather than
the exposure-shortening SLA.

- Repeated settings changes retained one WHEP quality connection generation;
  settings no longer caused media reconnects.
- Browser-visible media remained Live through the settings series.
- Final fixed-sequence production acceptance measured 5 s -> 200 ms at 624 ms
  and 30 s -> 100 ms at 481 ms from browser action to the exact target
  generation. Both remained below the approved normal thresholds.
- The same run observed the rendered Requested -> Applied -> Visible evidence
  progression, ended at Visible 500 ms with control released, and reported
  current partitions of 57/57 exact and 2/2 exact with zero unknown samples.
- Exact settings partitions contained zero unknown samples.
- No encoder replacement or camera reopen occurred during the final continuous
  capture series.

### Final closure qualification — 2026-08-06

The committed release was requalified through the Mac Playwright MCP against
the real camera and production relay before preparing a pull request.

At 50 ms, ISO 0, one visible viewer sustained 19.9 fps with 510 exact and 0
unknown samples over a 12-second measurement; p95 presentation latency was 186
ms. ObsCam process CPU averaged 57.0%, RSS was 40.53 MiB, and aggregate host CPU
idle averaged 78.1% with a 68.9% minimum.

Four independent, genuinely visible browser contexts then ran for the same
12-second window. Every video advanced for the complete interval, every viewer
remained Live, cadence was 19.9-20.0 fps, and all reported samples were exact
with zero unknown. The worst p95 was 192 ms, a 6 ms increase from the one-viewer
baseline against the 50 ms gate. ObsCam CPU averaged 55.5%, RSS was 43.17 MiB
(+2.64 MiB from the one-viewer ObsCam process), and aggregate host idle averaged
79.5% with a 73.4% minimum. A focused rerun measured the complete Pi media stack
(ObsCam, FFmpeg, and MediaMTX): RSS averaged 149.44 MiB with one viewer and
153.80 MiB with four, a 4.35 MiB increase against the 32 MiB gate. While all
four remained visible, the operator applied
50 ms -> 100 ms: the target became exact in 393 ms and all four videos kept
advancing and remained Live. The operator restored 50 ms afterward.

The binding shortening cases were repeated on the same release. The first
5 s -> 200 ms transition became exact in 865 ms. The first 30 s -> 100 ms
transition became exact in 546 ms; a deliberate repeat became exact in 647 ms
and, after the report partition settled, showed Live, 10.1 fps, 42 exact, 0
unknown, and 92 ms p95. Both remain within the accepted requested-exposure plus
1,000 ms hard gate.

Finally, the release held 50 ms and ISO 600 continuously for three minutes.
The source generation advanced by 3,596 and browser video advanced by 180.088
seconds. All thirteen 15-second checkpoints were Live, 19.7-20.0 fps, and
entirely exact with zero unknown samples. The local RTP socket's kernel drop
counter remained zero. RSS was unchanged at 43.65 MiB between early and final
samples; final CPU averaged 51.3%, aggregate host idle averaged 77.9%, and no
correlation invalidation, recovery, or process restart appeared in the log.
The original 500 ms, ISO 100, monochrome tuple was restored and control was
released after qualification.
