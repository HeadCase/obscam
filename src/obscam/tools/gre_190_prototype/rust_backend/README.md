# GRE-190 Rust media-backend prototype

**Throwaway architecture evidence.** This prototype asks whether one native
owner can capture full-resolution RAW8 into a fixed buffer pool and let
independent JPEG and H.264 consumers observe latest-frame semantics without
blocking each other or copying frames through Python.

Drive the pure ownership model interactively:

```bash
cargo run --manifest-path \
  src/obscam/tools/gre_190_prototype/rust_backend/Cargo.toml
```

Use `c` to publish a synthetic capture, `j/J` to start/complete JPEG work, and
`h/H` to start/complete H.264 work. The complete buffer, waiting-generation,
active-lease, and replacement state is redrawn after every action.

Validate the deployed camera's GRE-191 identity boundary without capturing:

```bash
cargo run --manifest-path \
  src/obscam/tools/gre_190_prototype/rust_backend/Cargo.toml -- \
  identity 'ZWO ASI662MC' 1d274e0920010900
```

Run the current 20-frame RAW8 ownership smoke test:

```bash
cargo run --manifest-path \
  src/obscam/tools/gre_190_prototype/rust_backend/Cargo.toml -- capture-smoke
```

The camera command requires exactly one exact model match and the enrolled
factory serial before capture. It opens only that candidate. It currently
configures 1920x1080 RAW8 video at 10 ms, gain 0, high-speed mode 1, and USB
bandwidth 100, then timestamps SDK completion and releases every frame through
both logical consumer leases.

Publish native-camera frames through the hardware H.264/WebRTC path after
starting the isolated MediaMTX configuration:

```bash
cargo run --manifest-path \
  src/obscam/tools/gre_190_prototype/rust_backend/Cargo.toml -- \
  h264-stream 60 10000 0 20
```

Arguments after `h264-stream` are duration seconds, exposure microseconds, gain,
and declared/paced output fps. An optional final `night` argument applies the
temporary closed-roof display stretch; it is exploratory presentation, not a
GRE-190 finalist. The gateway requires a read timeout longer than the maximum
exposure plus processing time. The H.264 worker consumes only its newest waiting
generation, so a slower Bayer conversion or hardware encoder cannot queue stale
camera frames or block the independent JPEG lease.
