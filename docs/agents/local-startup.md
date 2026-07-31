# Local Production Startup

This is the temporary manual startup procedure until GRE-224 and GRE-226
install independently supervised appliance services and qualified releases.
It runs the real ASI662MC, the release-mode Rust application, hardware H.264,
and the pinned MediaMTX binary. It is not a deterministic test stack.

## Prerequisites

- Run commands from the repository root.
- `/usr/local/bin/mediamtx` must report version `v1.19.3`.
- Its SHA-256 must match `deploy/mediamtx-linux-arm64.sha256`.
- `/etc/obscam/mediamtx.yml` is the installed host configuration. It matches
  the checked production media path while advertising only the permitted
  `wg0` and LAN addresses.
- The release binary must be built from the intended checkout.

Verify the installed relay:

```sh
mediamtx --version
sha256sum /usr/local/bin/mediamtx
sed -n '$p' deploy/mediamtx-linux-arm64.sha256
```

Build the browser assets and production application after changing either:

```sh
npm test
cargo build --release -p obscam
```

## Start

Use two terminals. Start MediaMTX first only for clearer logs; ObsCam does not
depend on startup ordering.

Terminal one runs:

```sh
mediamtx /etc/obscam/mediamtx.yml
```

The installed host configuration disables interface discovery and advertises
only `10.164.190.1` and `192.168.1.200`. MediaMTX must not inspect or advertise
protected `wg1` (`192.168.4.9`), and that interface is never a browser route or
fallback.

Terminal two runs the production application with its default real-camera
configuration:

```sh
RUST_LOG=info target/release/obscam
```

Do not set `OBSCAM_CAMERA_SOURCE=deterministic` for a production session.

## Open and verify

On the observatory LAN, open:

```text
http://192.168.1.200:8080
```

Over the permitted WireGuard route, open:

```text
http://10.164.190.1:8080
```

The local health contracts are:

```sh
curl --fail http://127.0.0.1:8080/api/v1/health
curl --fail http://127.0.0.1:8080/api/v1/runtime
```

MediaMTX should log that path `obscam` is online with one H.264 track. A
browser WHEP connection should return HTTP 201 and display native 1920×1080
video. The runtime relay field remains `not_observed` until a later accepted
relay-readiness contract replaces that bootstrap limitation; MediaMTX's stream
log and the browser-facing WHEP check are the current relay evidence.

## Stop

Press `Ctrl-C` once in the ObsCam terminal and once in the MediaMTX terminal.
Both are manual foreground processes until GRE-224 supplies supervision.
