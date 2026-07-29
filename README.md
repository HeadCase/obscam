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

The production `obscam` Rust service now boots without camera or media
dependencies and serves a truthful unavailable viewer. Its fixed-size,
RAM-only bootstrap state gives every process an explicit runtime epoch and
reports capture, encoder, and relay availability independently through
`/api/v1/runtime` and `/api/v1/health`.

The embedded browser application is compiled from vanilla TypeScript. Until a
trustworthy frame is exactly correlated, it displays `Unavailable` and leaves
frame age, cadence, source generation, and visible latency unknown.

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
