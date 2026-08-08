# ASIAIR synchronization operations

The repository owns `asiair-sync` as an independent host-integration service.
It copies completed files from the read-only ASIAIR SMB share to observatory
NFS storage. It is not a dependency of ObsCam, MediaMTX, AllSky, either mount,
or WireGuard.

The service is deliberately additive:

- it uses `rclone copy`, never `sync`, `move`, or a deletion command;
- it never deletes or modifies the ASIAIR source;
- it never deletes destination files; and
- it defers files modified within the last two minutes.

## Qualified topology

| Role | Mount | Remote | Service path |
| --- | --- | --- | --- |
| ASIAIR source | `/mnt/asiair` | `//192.168.1.201/TF Images` (read-only CIFS) | `/mnt/asiair/ASIAIR/` |
| NAS destination | `/mnt/library` | `192.168.19.12:/create/photos` (NFSv4) | `/mnt/library/astro/anser/` |

The source is the **contents** of `/mnt/asiair/ASIAIR/`, not the SMB share
root. Files therefore arrive directly as `log/...`, `Plan/...`, and similar
paths beneath `astro/anser`; an intermediary destination `ASIAIR` directory is
incorrect.

## Host prerequisites

Install the runtime packages:

```sh
sudo apt install coreutils rclone util-linux
```

Create an unprivileged, non-login `asiair-sync` account. On the qualified host
it has UID 1002 and primary GID 100. Numeric IDs are host configuration, not a
portable requirement: the actual requirement is that this identity can read
the CIFS source and create, inspect, and remove a test file at the NFS
destination.

The NAS export maps requests from the Pi's permitted client address to a NAS
identity with write access. Confirm the effective access from the service
identity before enabling the timer:

```sh
sudo -u asiair-sync test -r /mnt/asiair/ASIAIR
sudo -u asiair-sync touch /mnt/library/astro/anser/.asiair-sync-permission-test
sudo -u asiair-sync stat /mnt/library/astro/anser/.asiair-sync-permission-test
sudo -u asiair-sync rm /mnt/library/astro/anser/.asiair-sync-permission-test
```

Do not infer NFS authorization from a successful write as an interactive user;
test as `asiair-sync`.

## Automounts

Both endpoints must be systemd automounts. On the qualified host their
`/etc/fstab` entries are:

```fstab
192.168.19.12:/create/photos /mnt/library nfs4 rw,_netdev,nofail,x-systemd.automount,x-systemd.mount-timeout=15s,x-systemd.idle-timeout=2min 0 0
//192.168.1.201/TF\040Images /mnt/asiair cifs guest,ro,vers=3.0,iocharset=utf8,file_mode=0444,dir_mode=0555,_netdev,nofail,x-systemd.automount,x-systemd.mount-timeout=15s,x-systemd.idle-timeout=2min 0 0
```

The idle timeout is shorter than the five-minute sync interval so unused
mounts are released between cycles. A busy transfer keeps both mounts in use.
Do not use `mount -a` to test recovery: it mounts these entries directly
instead of exercising their generated automount units.

After editing `fstab`, reload and restart the automounts:

```sh
sudo systemctl daemon-reload
sudo systemctl restart mnt-asiair.automount mnt-library.automount
```

## Installation

The synchronization identity and host-specific configuration remain separate
from the camera-appliance installer. Install this independently supervised
worker with:

```sh
sudo install -d -o root -g root -m 0755 /etc/obscam /usr/local/libexec/obscam
sudo install -o root -g root -m 0755 deploy/asiair-sync /usr/local/libexec/obscam/asiair-sync
sudo install -o root -g root -m 0644 deploy/asiair-sync.env.example /etc/obscam/asiair-sync.env
sudo install -o root -g root -m 0644 deploy/systemd/asiair-sync.service /etc/systemd/system/asiair-sync.service
sudo install -o root -g root -m 0644 deploy/systemd/asiair-sync.timer /etc/systemd/system/asiair-sync.timer
sudo systemctl daemon-reload
```

Review `/etc/obscam/asiair-sync.env` before enabling the timer. It is
host-specific and contains no credentials. Confirm in particular that
`ASIAIR_SYNC_SOURCE` names `/mnt/asiair/ASIAIR`, while
`ASIAIR_SYNC_SOURCE_MOUNT` remains `/mnt/asiair`.

Run a dry run, review its relative paths, then enable the schedule:

```sh
sudo -u asiair-sync /usr/local/libexec/obscam/asiair-sync --dry-run /etc/obscam/asiair-sync.env
sudo systemctl enable --now asiair-sync.timer
```

Dry-run paths should begin with source contents such as `log/` or `Plan/`, not
`ASIAIR/`.

## Routine operation

The timer starts a fail-soft oneshot every five minutes after the previous run
becomes inactive. Runs never overlap. A large backlog can keep one run active
for hours; this is normal.

```sh
systemctl status asiair-sync.service asiair-sync.timer --no-pager
systemctl list-timers asiair-sync.timer --all --no-pager
sudo journalctl -u asiair-sync.service -n 100 --no-pager
```

Start an immediate cycle with:

```sh
sudo systemctl start asiair-sync.service
```

The qualified transfer policy is a 512 KiB/s bandwidth cap, one transfer, two
checkers, low CPU/I/O scheduling priority, and a four-hour service timeout.

## Failure and recovery

At the beginning of every cycle the worker triggers each automount and verifies
the exact CIFS or NFS layer and expected remote beneath autofs. A missing or
incorrect mount defers the cycle successfully so optional storage does not
produce a restart storm or block camera services.

During an active copy, the worker probes both endpoints every 30 seconds. If an
endpoint disappears it terminates rclone and leaves a visible failed service
run. The timer remains active and tries again later. Rclone's additive copy
semantics resume incomplete backlog work when both endpoints return.

Normal recovery requires no manual mount or service reset:

1. restore the ASIAIR or NAS/network path;
2. confirm the relevant automount is active; and
3. allow the next timer cycle to trigger and resume copying.

Use the verification procedure in
[`docs/verification/asiair-sync.md`](../verification/asiair-sync.md) when
qualifying recovery after host, mount, permission, or service changes.

## Troubleshooting

### Cycle says a path is not the expected mount

Inspect both the autofs trigger and the underlying remote filesystem:

```sh
systemctl status mnt-asiair.automount mnt-library.automount --no-pager
systemctl status mnt-asiair.mount mnt-library.mount --no-pager
findmnt --noheadings --mountpoint /mnt/asiair --output FSTYPE,SOURCE
findmnt --noheadings --mountpoint /mnt/library --output FSTYPE,SOURCE
```

When mounted, `findmnt` may show both the autofs layer and the underlying CIFS
or NFS layer. The worker deliberately validates the latter.

### Destination permission denied

Repeat the create/stat/remove test as `asiair-sync`. Correct authorization at
the NAS export; changing only local ownership or testing as another Pi user
does not prove the service identity can write through NFS identity mapping.

### Service remains activating

Check the journal for ongoing copies. The next timer timestamp is intentionally
blank while the oneshot is active because the next five-minute interval begins
when that run finishes.

### ASIAIR or NAS is routinely powered off

No intervention is required. After its two-minute idle timeout, the unavailable
mount returns to an automount waiting state. A later timer cycle retriggers it.
Do not add hard mount, VPN, or `network-online.target` dependencies to the sync
unit.
