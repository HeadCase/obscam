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

The production `obscam` Rust service now owns the continuously warm camera,
reconstructs default neutral monochrome directly from full-resolution RAW8,
and feeds one long-lived FFmpeg hardware-H.264 publication. Pinned MediaMTX
fans that stream directly to origin-aware WHEP browser sessions. The service
still boots its HTTP contracts truthfully when camera or media components are
unavailable.

The embedded browser application is compiled from vanilla TypeScript. It can
show the complete native frame without asserting frame identity; until
GRE-217 adds exact correlation, frame age, cadence, source generation, and
visible latency remain unknown.

See:

- `docs/agents/architecture.md` for binding local architecture guardrails
- `docs/agents/issue-tracker.md` for the Linear workflow

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
