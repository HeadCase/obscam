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

## Install and run

Install ObsCam on the qualified Raspberry Pi 4 host with the ASI662MC attached
and the cgroup v2 memory controller enabled. From a fresh repository checkout,
build the embedded browser and release binary:

```sh
npm ci
npm test
cargo build --release -p obscam
```

Supply the pinned MediaMTX v1.19.3 Linux ARM64 binary and ZWO SDK 1.41 library,
then install the complete qualified release. Copy and edit the host
configuration only when the local addresses or camera defaults differ:

```sh
cp deploy/host.env.example /tmp/obscam-host.env
obscam_release_id=obscam-$(git rev-parse --short HEAD)
sudo deploy/install-appliance install \
  --release-id "$obscam_release_id" \
  --obscam-binary target/release/obscam \
  --mediamtx-binary /path/to/mediamtx \
  --zwo-sdk-library /usr/local/lib/libASICamera2.so.1.41 \
  --host-config /tmp/obscam-host.env
sudo systemctl start obscam.target
```

Verify the application and open it from an approved browser route:

```sh
systemctl is-active obscam.target obscam.service obscam-mediamtx.service
curl --fail http://127.0.0.1:8080/api/v1/health
```

Browse to `http://192.168.1.200:8080` on the observatory LAN or
`http://10.44.0.1:8080` over the permitted ObsCam WireGuard route. See the
[qualified release guide](docs/agents/qualified-releases.md) for host
preflight, migration, rollback, and diagnostics.

## Current status

The production `obscam` Rust service now runs as an independently supervised,
least-privilege appliance service and owns the continuously warm camera,
reconstructs default neutral monochrome directly from full-resolution RAW8,
and feeds one long-lived FFmpeg hardware-H.264 publication through a bounded
local RTP observer. Rust relays that unchanged stream over a private veth to
pinned MediaMTX in its dedicated network namespace; exact-address translation
exposes WHEP only on the approved ObsCam service addresses. MediaMTX fans the
stream directly to origin-aware browser sessions. The service
still boots its HTTP contracts truthfully when camera or media components are
unavailable.

Operators manage the complete appliance through `obscam.target`; ObsCam,
MediaMTX, and the isolated media network remain independently supervised
members beneath that single lifecycle unit.

The embedded browser application is compiled from vanilla TypeScript. It
matches `requestVideoFrameCallback` RTP metadata only against bounded,
runtime/stream-epoch-scoped mappings broadcast by Rust. Missing, stale,
evicted, reset, fractional, ambiguous, or conflicting evidence remains
unknown; arrival order and the newest server frame are not substitutes.

The restart camera tuple defaults to 500 ms exposure, gain 100, and monochrome.
It can be changed with `OBSCAM_DEFAULT_EXPOSURE_MS`, `OBSCAM_DEFAULT_GAIN`, and
`OBSCAM_DEFAULT_TREATMENT`; startup fails closed unless the values are one of
the curated exposure choices, a 0–600 gain detent in steps of 50, and either
`monochrome` or `colour`.

See:

- `docs/agents/architecture.md` for binding local architecture guardrails
- `docs/agents/qualified-releases.md` for managed appliance installation,
  activation, rollback, and diagnostics
- `docs/agents/issue-tracker.md` for the Linear workflow
- `docs/operations/asiair-sync.md` for ASIAIR synchronization operations and
  recovery
- `docs/operations/shared-host.md` for resource policy and read-only diagnostics
- `docs/verification/asiair-sync.md` for ASIAIR synchronization acceptance and
  deployed evidence
- `docs/verification/gre-225-shared-host.md` for shared-host qualification

## Quality gate

Every code change must pass:

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

Build and test the browser assets before compiling the Rust binary:

```text
npm ci
npm test
```
