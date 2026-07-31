# GRE-218 service-quality verification

## Implemented contract

Rust retains browser presentation evidence in runtime-local RAM. The fixed
limits are 16 least-recently-active clients and 512 samples per client, and both
limits are returned in every evidence response. A tab-scoped UUID identifies a
client. The server issues an explicit monotonically increasing media connection
generation for that client; later generations increment reconnects, clear the
browser callback ordinal baseline, and keep prior samples in separate
compatibility partitions.

Every browser callback reports its connection generation, current stream epoch
when known, visibility, callback `presentedFrames` ordinal, calibrated
presentation time, clock uncertainty, and whether browser-side RTP correlation
was exact or unknown. Exact claims are independently resolved against Rust's
currently retained runtime/stream/RTP mapping. Missing, evicted, conflicting,
or mismatched mappings become unknown. Unknown samples retain only the facts
that remain known and never receive source/settings identity, treatment,
dimensions, or latency; they retain the browser's measured clock uncertainty.

Compatible samples partition by client connection, runtime epoch, stream epoch,
settings generation, treatment, native dimensions, and visibility. Source
generation remains per-frame identity and deduplicates repeats for unique
presented cadence. Gaps in the browser's monotonic `presentedFrames` counter
increment presentation skips. Latency and clock-uncertainty distributions use
nearest-rank min/p50/p95/p99/max over retained samples.

`GET /api/v1/service-quality` returns all retained clients plus the combined
aggregate. Its optional `clientId` query returns the same response shape scoped
to one browser. The browser reads that response at 2 Hz, renders only values
from it, and downloads the same parsed response as JSON. Presentation reports
are sent in bounded batches of at most 32 observations on a 250 ms cadence.
The browser retains at most 32 pending observations and drops the oldest under
backpressure; skipped callback ordinals remain explicit and are counted by the
server. This caps reporting traffic and memory without increasing media latency.

## Automated evidence

- Rust unit tests cover nearest-rank boundaries, 16-client LRA eviction,
  512-sample retention, client separation, reconnect fencing, settings,
  treatment and visibility partitions, callback skips, server-validated exact
  mappings, unknown correlation, unique cadence, latency and uncertainty.
- HTTP contract tests cover clock calibration, explicit connection generations,
  stale-generation rejection, unknown-sample nullability, response limits,
  client-scoped and combined aggregates, and production asset delivery.
- TypeScript tests validate fixed limits, scoped client identity, aggregate
  arithmetic, bounded 32-observation batching, authoritative rendering, and
  malformed-response rejection.

## Browser and deployed-stack evidence — 2026-07-31

The Mac Playwright browser reached the release Rust service over the permitted
`http://10.164.190.1:8080` WireGuard route. Runtime and clock reads succeeded,
the new service-quality connection request returned HTTP 200, and a tab-scoped
UUID was accepted on the appliance's non-secure HTTP origin. This check caught
and corrected an initial dependency on `crypto.randomUUID()`, replacing it with
an RFC 4122 v4 UUID generated through `crypto.getRandomValues()`.

The checked-in MediaMTX configuration had interface discovery enabled despite
the startup guide claiming otherwise. Candidate advertisement from discovered
interfaces is now disabled, and the only static browser hosts are
`10.164.190.1` and `192.168.1.200`. The namespace described below is the
independent enforcement boundary for Pion's unavoidable interface enumeration.

That failure was resolved without granting host interface visibility. MediaMTX
now runs in `obscam-media`, containing only `lo` and the private `media0` veth.
Pion receives netlink inside that namespace. Exact-destination rules translate
WHEP TCP/8889 and ICE UDP/8189 only for `10.164.190.1` and `192.168.1.200`; Rust
relays unchanged observed RTP/RTCP to `169.254.218.2`. Forwarding permits
established service flows and new ICE only toward the approved browser networks;
other namespace forwarding and host access are dropped. Deployment tests cover
the namespace, addresses, ports, owned-veth matching, and fail-closed policy.

The real ASI662MC capture and hardware encoder reported ready. MediaMTX v1.19.3
reported the H.264 stream online with one track, established peer connections,
and remained active with zero restarts. On the Mac browser, WHEP returned HTTP
201, video played at native 1920×1080, and the rolling display accumulated both
exact and conservative unknown samples. The service-quality endpoint returned
HTTP 200 with the fixed 16-client/512-sample limits, and the JSON download
completed. Desktop 1440×900 and mobile 390×844 layouts had no horizontal
overflow. The pre-existing `/favicon.ico` 404 remained the only current-page
console error after the process was stable.

Review-driven recovery verification then stopped and recreated only the owned
namespace and MediaMTX unit while the real Rust camera/encoder process remained
running. Native video and advancing service-quality samples resumed without
restarting the camera owner. A request initiated inside the namespace toward
the host service gateway was blocked, while WHEP/ICE still passed through the
tightened forwarding policy.

Both approved destination addresses completed WHEP and video playback. The
`192.168.1.200` attempt nevertheless produced a WireGuard-range peer-reflexive
source in MediaMTX, proving that the Mac routed it through WireGuard. It verifies
the exact LAN destination mapping but not physical-LAN ingress. A device
actually attached to the observatory LAN remains the required Andrew-path check;
this is an environment gap, not a substitute failure for GRE-218's implemented
service-quality contract.
