# GRE-224 appliance-service verification

## Implemented contract

ObsCam runs as the static non-login `obscam` identity. MediaMTX retains its
separate stateless `DynamicUser` identity inside the dedicated media namespace.
Neither service orders itself after the other or after remote readiness.

The exact ASI662MC USB product `03c3:662b` is `root:obscam-camera 0660`; Rust
continues to validate exactly one model and the factory serial before opening
the SDK owner. The named Raspberry Pi `bcm2835-codec-encode` node is
`obscam:video 0660`; other V4L2 nodes retain `root:video`. The service device
cgroup admits USB and V4L2 classes, while filesystem ownership limits which
nodes the identity can open. `AF_NETLINK` is retained because a transient-unit
probe proved libusb initialization fails with error `-99` without it.

Both long-running services restart after five seconds on every unexpected exit,
including a clean exit status, with `StartLimitIntervalSec=0`. The namespace
setup retries on failure as an
independent oneshot. No systemd watchdog is present. Managed output goes only
to journald with per-unit rate limits.

MediaMTX startup verifies the private binary and configuration hashes, exact
`v1.19.3` version, and exactly one `moq: no` setting before the process starts.
The unit never executes the ambient `/usr/local/bin/mediamtx` path.

## Automated evidence

- Rust deployment-contract tests cover independent ordering, identities,
  restart policy, logging, namespace ownership, exact device rules, hardening,
  pin verification, and MediaMTX exposure boundaries.
- `deploy/tests/install-appliance-test` exercises a clean staged installation,
  exact idempotent reinstall, non-login identity creation, unit enablement,
  conflicting-path refusal, and pin rejection before target mutation.
- The installer test is invoked from the Rust deployment test target so the
  normal workspace suite cannot omit it.

The following repository gates passed:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo deny check
cargo machete
npm test
npm run typecheck:browser
```

`cargo deny check` reported only the repository's accepted duplicate-version
warnings. `shellcheck` was unavailable on the Pi; the POSIX installer and pin
verifier were exercised through the staged installer integration test instead.
No browser behavior changed, so the operator-Mac graphical suite was not an
applicable GRE-224 gate.

## Deployed Pi evidence

Status: **passed on 2026-08-07**.

The installer accepted the reviewed release ObsCam binary and the existing
MediaMTX v1.19.3 binary only after its pinned checksum matched. Installed file
verification and `systemd-analyze verify` passed. The final live permissions
were:

```text
/dev/bus/usb/002/003 mode=660 owner=root group=obscam-camera
/dev/video11 mode=660 owner=obscam group=video
/dev/video10 mode=660 owner=root group=video
```

AllSky remained active under the same main PID with zero restarts throughout
installation and permission validation. A transient service probe proved the
camera SDK boundary requires `AF_NETLINK`; the same probe succeeded in listing
both cameras once that family was admitted without changing device access.

The Pi rebooted with its remote ASIAIR mount unavailable and reached its
pre-existing degraded host state without blocking the appliance services.
ObsCam and the namespace owner started at local boot readiness; MediaMTX
started independently three seconds later. ObsCam retained capture and encoder
ownership while the relay was absent, then observed the returning relay without
restarting. The local health and runtime contracts reported capture, encoder,
and relay `ready`. Exact camera and encoder permissions persisted across the
reboot. AllSky started normally with zero restarts.

A controlled `SIGKILL` of MediaMTX produced a five-second supervised delay and
one MediaMTX restart. ObsCam kept the same PID, capture and hardware encoding
remained ready, and relay readiness returned after the new MediaMTX process
published the same H.264 path. AllSky remained unchanged.

A controlled `SIGKILL` of ObsCam then produced a five-second supervised delay
and one ObsCam restart. MediaMTX kept the same PID. The new Rust process
revalidated and reclaimed the exact ASI662MC, restored hardware encoding, and
reported capture, encoder, and relay `ready`; AllSky again remained unchanged
with zero restarts. Neither unit reached a start limit, and no watchdog or
cross-service restart coupling was involved.

Final review also sent each main process `SIGTERM`, which systemd classifies as
a clean exit for these long-running services. `Restart=always` restarted each
after five seconds with `ExecMainStatus=0`; the other service kept its PID,
health returned fully ready, and AllSky retained PID 1427 with zero restarts.

At final review, the deployed processes used approximately 40 MiB RSS each.
The checked-in service policy therefore favors both interactive workloads with
relative CPU/I/O weights of 200, protects them from OOM selection ahead of the
`asiair-sync` worker, and sets generous high/max memory thresholds of
384/512 MiB for ObsCam and 256/384 MiB for MediaMTX.
