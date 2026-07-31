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
dimensions, latency, or uncertainty.

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
are serialized with at most one replaceable pending observation, preventing a
slow diagnostic request from growing a browser-side queue; skipped callback
ordinals remain explicit.

## Automated evidence

- Rust unit tests cover nearest-rank boundaries, 16-client LRA eviction,
  512-sample retention, client separation, reconnect fencing, settings,
  treatment and visibility partitions, callback skips, server-validated exact
  mappings, unknown correlation, unique cadence, latency and uncertainty.
- HTTP contract tests cover clock calibration, explicit connection generations,
  stale-generation rejection, unknown-sample nullability, response limits,
  client-scoped and combined aggregates, and production asset delivery.
- TypeScript tests validate fixed limits, scoped client identity, aggregate
  arithmetic, authoritative rendering, and malformed-response rejection.

## Browser evidence and environment limitation — 2026-07-31

The Mac Playwright browser reached the release Rust service over the permitted
`http://10.164.190.1:8080` WireGuard route. Runtime and clock reads succeeded,
the new service-quality connection request returned HTTP 200, and a tab-scoped
UUID was accepted on the appliance's non-secure HTTP origin. This check caught
and corrected an initial dependency on `crypto.randomUUID()`, replacing it with
an RFC 4122 v4 UUID generated through `crypto.getRandomValues()`.

Complete media-presentation evidence could not be rerun safely. Although the
installed MediaMTX v1.19.3 binary matched the repository checksum and was
started with the prescribed host configuration, its runtime output advertised
the protected `wg1` address contrary to the repository's documented
expectation. ObsCam and MediaMTX were stopped immediately; that interface and
configuration were not inspected or modified. Consequently, live exact and
unknown callback reporting, rendered rolling values, JSON download interaction,
and mobile viewport behavior remain blocked graphical checks rather than
inferred passes. The pre-existing `/favicon.ico` 404 was also present.
