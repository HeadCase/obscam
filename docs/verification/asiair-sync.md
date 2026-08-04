# ASIAIR synchronization verification

This document defines acceptance checks and records deployed evidence for the
repository-owned `asiair-sync` host integration. Routine installation,
operation, recovery, and troubleshooting live in
[`docs/operations/asiair-sync.md`](../operations/asiair-sync.md).

## Repository checks

Run the focused worker tests and syntax checks:

```sh
sh -n deploy/asiair-sync deploy/tests/asiair-sync-test
deploy/tests/asiair-sync-test
git diff --check
```

The tests must prove exact copy arguments, absence of destructive operations,
safe deferral for unavailable or incorrect mounts, source and destination
containment, preservation of rclone failures, and endpoint-watchdog behavior.

## Host dry run

Run the installed worker as its service identity:

```sh
sudo -u asiair-sync \
  /usr/local/libexec/obscam/asiair-sync \
  --dry-run \
  /etc/obscam/asiair-sync.env
```

Confirm that:

- both genuine remote filesystems mount beneath their autofs triggers;
- proposed paths begin directly with source contents such as `log/` and
  `Plan/`, never an intermediary `ASIAIR/`;
- the destination is `/mnt/library/astro/anser/`;
- recently modified files are excluded; and
- output contains additive copy actions only.

Start one live cycle only after reviewing the dry-run output:

```sh
sudo systemctl start asiair-sync.service
systemctl status asiair-sync.service asiair-sync.timer --no-pager
sudo journalctl -u asiair-sync.service -n 100 --no-pager
```

Confirm at least one full FITS file arrives at the direct destination with the
expected size and that the source remains unchanged.

## Recovery qualification

1. Start with both automounts waiting and both mounts inactive.
2. Run a successful cycle and confirm exact CIFS and NFS mounts were triggered.
3. Shut down ASIAIR during a FITS transfer. Confirm the endpoint watchdog stops
   the active copy with a visible failure and does not continue walking a stale
   backlog.
4. After the CIFS mount idles out, confirm another cycle defers without invoking
   rclone or writing into the bare mount-point directory.
5. Restore ASIAIR and confirm a later timer cycle mounts it and resumes copying
   without a service restart or manual mount.
6. Repeat the loss-and-restoration test for the NAS. Confirm the active copy
   stops, then the next eligible timer cycle resumes after access returns.
7. Reboot with at least one remote resource absent. Confirm boot is not blocked,
   both automounts return active and waiting, and their unavailable underlying
   mounts remain inactive.
8. Restore resources independently and confirm synchronization begins only once
   both exact mounts are available.

Unavailable resources are normal and produce a successful deferred cycle.
Incorrect mount identities are also deferred to protect local storage. Genuine
rclone failures remain visible as failed service runs while the timer continues
to schedule later attempts.

Do not inspect, reconfigure, or couple the worker to `wg1` while qualifying NAS
loss and recovery. Treat the VPN as independent infrastructure and manipulate
only the endpoint availability chosen for the test.

## Deployed evidence

The rebuilt `osprey` host passed the following checks on 2026-08-04 and
2026-08-05:

- the installed dry run triggered both automounts, validated the exact CIFS and
  NFS layers beneath autofs, and proposed additive copies only;
- a live run copied logs, thumbnails, and full 17.268 MiB FITS files at the
  configured 512 KiB/s cap;
- powering down ASIAIR during a FITS transfer produced a checksum failure and
  removed the failed destination copy;
- the endpoint watchdog then stopped the stale rclone walk with a visible
  service failure;
- a cycle with ASIAIR still absent deferred successfully without invoking
  rclone, and the inactive CIFS mount later recovered automatically;
- after ASIAIR returned, the timer remounted it and copied the previously
  interrupted FITS file without manual service or mount recovery;
- removing the QNAP NFS permission for the Pi caused the destination watchdog
  to stop the active cycle;
- restoring that permission allowed the next five-minute timer cycle to resume
  full FITS copies without manual recovery;
- rebooting with ASIAIR unavailable and the NAS available did not block boot:
  both automount units returned active and waiting while their underlying mounts
  remained inactive; and
- after correcting the qualified source to `/mnt/asiair/ASIAIR/`, an installed
  dry run proposed `log/...` and `Plan/...` paths directly beneath the
  destination, and a live run copied multiple full FITS files as `Plan/...`
  without creating another intermediary `ASIAIR` directory.

An intermediary destination directory created during pre-correction validation
was retained until the corrected additive backlog completed, then removed
manually by the operator on 2026-08-05. The service performed no migration or
deletion.
