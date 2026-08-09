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

Installation records the candidate and clean, managed, or GRE-225-adoption
mode before it changes host configuration, bootstrap links, or adopted paths.
An interrupted installation resumes only for that exact candidate. Activation
then records the candidate, active set, and older rollback candidate before it
atomically advances relative `active` and `previous` links. The bounded live
gate verifies service state, local health at the configured bind address and
port, exact served browser assets, MediaMTX publication, and both independent
supervised restart directions. Any failure restores both prior links and runs
the former set's complete live gate. Persistent markers expose interrupted
installation and activation state read-only and are recovered before the next
install or rollback mutation.

`obscam-release status` reports active, previous, pending activation, and
pending installation state without mutation. `obscam-release rollback`
validates and swaps the two complete sets, then applies the same live gate.

## Automated evidence

`deploy/tests/install-appliance-test` covers clean installation, idempotence,
upgrade, immutable and exact manifests, incompatible relay/SDK/host
configuration, malformed bind addresses, host-path and identity conflicts,
explicit GRE-225 adoption, automatic rollback, manual rollback, truthful
status, interrupted installation/adoption recovery, interrupted activation
recovery, traversal-resistant release links, and retained recovery markers
when restoration cannot be qualified. The Rust deployment contract ensures
the services and helpers consume only the active release and that the
post-activation gate contains every required bounded public check.

The installed-command fixtures additionally invoke upgrades through the stable
`/usr/local/sbin/obscam-release` symlink, verify the resolved active source set
before copying it, refuse source drift without switching, and create
operator-readable immutable release evidence under a restrictive root umask.

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

Status: **passed on 2026-08-09**.

The live `osprey` Raspberry Pi qualified the real ASI662MC (`03c3:662b`), ZWO
SDK 1.41, hardware H.264 encoder, MediaMTX v1.19.3 namespace, systemd
supervision, and Mac Chromium over the approved `wg0` route. GRE-225 was
explicitly adopted into `gre-226-467b839-a`; later qualified sets were upgraded
through the installed release manager.

The gate exercised:

- a deliberately non-starting candidate, which timed out at local health and
  automatically restored `gre-226-432e805-b` with no pending journals;
- a successful installed-command upgrade to `gre-226-432e805-c` and an
  explicit manual rollback to `gre-226-432e805-b`;
- final activation of `gre-226-407bf2c-d`, with
  `gre-226-432e805-b` retained as the previous set;
- full installed verification, exact active executable paths, ready health and
  MediaMTX publication, independent service restarts, camera permission
  recovery, and operator-readable diagnostics;
- unchanged `/etc/systemd/system/allsky.service` ownership, metadata, and
  SHA-256 (`542f48082c8eb17a277342caf4c910f28096cb07e88bb0e7e12b4ab0ab2f83b4`);
- desktop `1440x900` and mobile `390x844` Chromium checks at
  `http://10.44.0.1:8080`: Live, advancing 1920x1080 video, WHEP HTTP 201,
  browser API HTTP 200, no viewport overflow, console errors, or failed
  requests.

The live gate found and drove regression coverage for two defects before final
qualification: installed-manager source-layout resolution and root-umask
permissions on release metadata. Both fixes passed the full repository gates
and independent Standards and Spec re-review before deployment.
