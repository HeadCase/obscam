# GRE-225 shared-host verification

## Repository checks

The deployment contract tests prove the relative weight and OOM ordering,
absence of CPU quotas, real-time scheduling, and viewer-aware throttling, and
the generous evidence-derived ObsCam/MediaMTX memory thresholds. The staged
installer test proves ownership, idempotence, and conflict refusal for the
AllSky drop-in and diagnostic command.

The diagnostic fixture proves that output is bounded to 100 journal records,
only `wg0` is inspected, no service/mount mutation is issued, and a failed
health check does not suppress later thermal evidence. Existing ASIAIR tests
prove exact mount validation, safe missing-resource deferral, bounded transfer
policy, and active-copy endpoint loss handling.

Run:

```sh
sh -n \
  deploy/install-appliance \
  deploy/obscam-diagnostics \
  deploy/tests/install-appliance-test \
  deploy/tests/obscam-diagnostics-test
deploy/tests/install-appliance-test
deploy/tests/obscam-diagnostics-test
deploy/tests/asiair-sync-test
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo deny check
cargo machete
npm test
npm run typecheck:browser
git diff --check
```

Repository status: **passed on 2026-08-08**. All listed checks passed.
Additionally, `shellcheck` was unavailable on the Pi; `sh -n`, the staged
installer fixture, and both shell behavior suites passed. `cargo deny check`
emitted only the repository's accepted duplicate-version warnings.
`systemd-analyze verify` also accepted the appliance and synchronization units.

Running the uninstalled diagnostic command read-only against the live host
reported ready ObsCam health, active AllSky and `wg0` units, genuine CIFS/NFS
mounts beneath autofs, bounded recent logs, 67.6 C, and `get_throttled=0x0`.
The unprivileged operator could not read WireGuard interface details; that
section reported status 1 and the remaining resource and thermal sections
still completed, as designed. No host files or services were changed.

## Deployed Pi gate

This gate requires the real ASI662MC owner, AllSky's ASI178MC owner, the pinned
MediaMTX relay, `wg0`, genuine optional mounts, rclone, and four browser viewers.
Substitutes cannot complete it.

1. Install the reviewed build, reload systemd, and verify the installed
   appliance contract. Record both camera-service PIDs and restart counts.
2. Run `obscam-diagnostics` once with the optional mounts available and once
   while an endpoint is absent. Confirm both reports complete, logs contain no
   more than 100 records, and the absent resource remains inactive rather than
   blocking either camera owner.
3. Start four advancing browser viewers over the approved LAN/`wg0` routes and
   start a genuine bounded `asiair-sync` transfer while AllSky remains active.
4. During sustained combined load, record service resource properties,
   `/api/v1/health`, AllSky state/restarts, `wg show wg0`, viewer advancement,
   temperature, and `get_throttled` before and after the interval.
5. Confirm ObsCam and MediaMTX remain useful, AllSky and `wg0` remain healthy,
   rclone retains 512 KiB/s with one transfer/two checkers and yields under
   contention, and no camera service restarts because an optional component
   fails.
6. Remove and restore each optional endpoint independently. Confirm sync
   defers or stops as qualified, then resumes on a later timer cycle without
   restarting ObsCam, MediaMTX, AllSky, or WireGuard.

## Evidence status

GRE-224 measured approximately 40 MiB ObsCam RSS and 42 MiB MediaMTX RSS and
qualified the retained 384/512 MiB and 256/384 MiB thresholds. GRE-271 already
qualified missing ASIAIR/NAS behavior and the bounded synchronization policy.

The GRE-225 four-viewer combined-load run must be recorded here after the
reviewed branch is installed. Until that hardware gate passes, GRE-225 is not
fully qualified for completion.
