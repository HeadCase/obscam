# ObsCam

ObsCam is being specified and rebuilt as a greenfield, low-latency observatory
monitor for the Raspberry Pi 4 and ZWO ASI662MC.

The accepted direction is one continuously warm Rust camera owner acquiring
full-resolution RAW8 frames, applying neutral monochrome or colour treatment,
and orchestrating FFmpeg hardware H.264 encoding. MediaMTX independently fans
that single stream out to browsers over WHEP/WebRTC. Runtime state remains in
RAM and the browser owns interaction and presentation.

The legacy Python/JPEG application and executable exploration prototypes have
been removed. Git history preserves them when historical investigation is
explicitly required; they are not implementation precedent.

## Current status

The architecture is being completed in the GRE-179 Linear map. The repository
currently retains the evidence needed to finish that work and will gain a new
Rust workspace as production implementation begins.

See:

- `docs/agents/architecture.md` for binding local architecture guardrails
- `docs/agents/issue-tracker.md` for the Linear workflow
- `research/` for retained experimental evidence

## Quality gate

Once the Rust workspace is introduced, every code change must pass:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo deny check
cargo machete
```

Browser checks and hardware-gated Pi acceptance are additionally required when
relevant. Hardware-dependent verification must report unavailable equipment as
blocked rather than infer success from substitutes.

## Browser service-quality evidence

The production-shaped probe accepts exact presentation callbacks through
`POST /api/presentations`. `POST /api/connections` advances a client's media
connection generation; the first connection is generation one and later
generations count as reconnects. Invalid runtime or stream epochs are rejected.

Evidence is RAM-only and bounded to 16 least-recently-active clients with 512
samples per client. Both limits are returned in every evidence response. The
oldest sample is evicted when a client window is full, and the least recently
active client is evicted when the client limit is reached.

`GET /api/evidence` builds the full agent-readable snapshot on demand.
`GET /api/evidence/clients/{client_id}` builds only one client's snapshot and is
the endpoint polled by the browser UI, preventing routine viewer polling from
cloning and sorting every viewer's retained samples. The UI and its evidence
download use that response directly rather than recalculating metrics.

Samples retain runtime and stream epochs, connection, source and settings
generations, treatment, dimensions, visibility, correlation status, latency,
and clock uncertainty. Aggregates are partitioned by compatibility boundaries:
runtime, stream and connection epochs, settings generation, treatment,
dimensions, and visibility. Source generation remains per-frame identity so a
cadence window can span successive frames. Percentiles use the nearest-rank
method over the bounded window; unknown correlations contribute to counts and
cadence but never receive invented latency or frame identity.

## MediaMTX deployment pin

ObsCam requires MediaMTX `v1.19.3` on 64-bit ARM. The binary and the checked-in
configuration are one deployment contract; other MediaMTX versions are not
accepted implicitly.

Install the checksum-pinned official ARM64 release:

```text
sudo scripts/install-mediamtx
```

The installer rejects non-ARM64 Linux hosts, verifies the release archive
against the pinned SHA-256 digest, installs `/usr/local/bin/mediamtx`, and
verifies the reported version. An alternate destination can be passed as the
first argument for non-system qualification. Upgrades preserve the displaced
binary as `mediamtx.previous` and refuse to overwrite an existing backup.

Exercise the production-shaped Rust, FFmpeg, RTP, MediaMTX, and WHEP seam:

```text
scripts/check-deployed-stack
```

The smoke test rejects version drift, starts the synthetic Rust pipeline and
the checked-in MediaMTX configuration, waits for the `obscam` H.264 path, checks
the WHEP endpoint, and proves that the prohibited MoQ path remains disabled.
