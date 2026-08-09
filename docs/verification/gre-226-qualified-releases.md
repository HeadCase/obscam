# GRE-226 qualified-release verification

## Implemented contract

Each release is a fixed compatibility set beneath
`/opt/obscam/releases/<release-id>`. Its strict checksum inventory covers the
Rust application, independently verifiable production browser assets,
MediaMTX v1.19.3 and its configuration, systemd definitions, deployment
helpers, and the pinned ZWO SDK 1.41 library and ABI link. Files and directories
are made non-writable before staging is renamed into its final versioned path.

Host configuration remains a separately validated regular file at
`/etc/obscam/host.env`. Installer-owned host paths are narrow stable symlinks
through `/opt/obscam/active`; an ownership marker distinguishes managed paths
from ambient files. A known GRE-225 installation can be migrated only through
the explicit `--adopt-gre-225` operation after its identities, inputs, relay
pin, and service paths validate. Replaced definitions remain recoverable with
the `.pre-gre-226` suffix.

Activation records the candidate, active set, and older rollback candidate,
then atomically advances relative `active` and `previous` links. The bounded
live gate verifies service state, local health, exact served browser assets,
MediaMTX publication, and both independent supervised restart directions. Any
failure restores both prior links and restarts the former set. A persistent
marker exposes an untrappably interrupted switch read-only and is recovered
before the next install or rollback mutation.

`obscam-release status` reports active, previous, and pending state without
mutation. `obscam-release rollback` validates and swaps the two complete sets,
then applies the same live gate.

## Automated evidence

`deploy/tests/install-appliance-test` covers clean installation, idempotence,
upgrade, immutable and exact manifests, incompatible relay/SDK/host
configuration, host-path and identity conflicts, explicit GRE-225 adoption,
automatic rollback, manual rollback, truthful status, and interrupted
activation recovery. The Rust deployment contract ensures the services and
helpers consume only the active release and that the post-activation gate
contains every required bounded public check.

The following repository gates passed on 2026-08-09:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo deny check
cargo machete
npm run typecheck:browser
npm test
deploy/tests/install-appliance-test
deploy/tests/obscam-diagnostics-test
deploy/tests/verify-memory-controller-test
deploy/tests/asiair-sync-test
systemd-analyze --root=<isolated-root> verify ...
git diff --check
```

The unrestricted Rust rerun passed after the sandbox denied the two tests that
bind local UDP sockets. `cargo deny check` emitted only the repository's
accepted duplicate-version warnings. `shellcheck` was unavailable; every
changed POSIX script passed `sh -n` and its behavioral fixture. No browser
behavior changed, so graphical browser verification was not an applicable
GRE-226 gate.

## Deployed Pi gate

Status: **not yet run**.

The final appliance gate requires the real ASI662MC, official ZWO SDK, hardware
H.264 encoder, MediaMTX namespace, systemd supervision, and browser/media
contracts. Install a reviewed release over the qualified GRE-225 predecessor;
capture `status` and diagnostics; exercise a successful upgrade, a deliberately
failed candidate with automatic restoration, and explicit manual rollback.
For every switch confirm exact assets and versions, health, publication,
independent service recovery, unchanged AllSky ownership, and truthful active
and previous release IDs. Unavailable hardware evidence is not replaced by a
fixture.
