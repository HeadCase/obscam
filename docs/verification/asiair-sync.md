# ASIAIR synchronization

The repository owns the independent `asiair-sync` host-integration policy. It
copies completed files from the read-only ASIAIR SMB share to observatory NFS
storage. It is not a dependency of ObsCam, MediaMTX, AllSky, either mount, or
WireGuard.

The qualified source is `/mnt/asiair/ASIAIR/`, not the SMB share root. Its
contents are copied directly into `/mnt/library/astro/anser/`; the destination
must not gain an intermediary `ASIAIR` directory.

## Host prerequisites

Install the runtime packages:

```sh
sudo apt install coreutils rclone util-linux
```

Create an unprivileged, non-login `asiair-sync` account whose NAS mapping has
already been qualified. Both endpoints must be systemd automounts. On the
qualified host their `/etc/fstab` entries are:

```fstab
192.168.19.12:/create/photos /mnt/library nfs4 rw,_netdev,nofail,x-systemd.automount,x-systemd.mount-timeout=15s,x-systemd.idle-timeout=2min 0 0
//192.168.1.201/TF\040Images /mnt/asiair cifs guest,ro,vers=3.0,iocharset=utf8,file_mode=0444,dir_mode=0555,_netdev,nofail,x-systemd.automount,x-systemd.mount-timeout=15s,x-systemd.idle-timeout=2min 0 0
```

The idle timeout is shorter than the five-minute sync interval so idle mounts
are released between cycles. A busy transfer keeps both mounts in use. Do not
use `mount -a` to test recovery: it mounts these entries directly instead of
starting their generated automount units.

## Manual installation

This temporary installation procedure remains in force until GRE-224 provides
the managed appliance installer:

```sh
sudo install -d -o root -g root -m 0755 /etc/obscam /usr/local/libexec/obscam
sudo install -o root -g root -m 0755 deploy/asiair-sync /usr/local/libexec/obscam/asiair-sync
sudo install -o root -g root -m 0644 deploy/asiair-sync.env.example /etc/obscam/asiair-sync.env
sudo install -o root -g root -m 0644 deploy/systemd/asiair-sync.service /etc/systemd/system/asiair-sync.service
sudo install -o root -g root -m 0644 deploy/systemd/asiair-sync.timer /etc/systemd/system/asiair-sync.timer
sudo systemctl daemon-reload
sudo systemctl enable --now asiair-sync.timer
```

Review `/etc/obscam/asiair-sync.env` before enabling the timer. It is deliberately
host-specific and contains no credentials.

## Verification

Run the repository test and a host dry run:

```sh
deploy/tests/asiair-sync-test
sudo -u asiair-sync /usr/local/libexec/obscam/asiair-sync --dry-run /etc/obscam/asiair-sync.env
```

Start one live cycle only after reviewing the dry-run output:

```sh
sudo systemctl start asiair-sync.service
systemctl status asiair-sync.service asiair-sync.timer --no-pager
sudo journalctl -u asiair-sync.service -n 100 --no-pager
```

The service uses `rclone copy`, never `sync`, `move`, or a deletion command.
Files modified within the last two minutes are deferred. Runtime is limited to
four hours; an interrupted backlog resumes during a later cycle. While rclone
runs, the worker checks both endpoints every 30 seconds. A failed bounded check
terminates the copy cycle so an unavailable device cannot leave rclone walking
a stale, pre-enumerated backlog until the service timeout.

For deployed recovery evidence:

1. Start with both automounts waiting and both mounts inactive.
2. Run a successful cycle and confirm exact CIFS and NFS mounts were triggered.
3. Shut down ASIAIR, wait for the CIFS mount to idle out, and run another cycle.
   Confirm an active cycle stops after its endpoint check, then confirm the next
   cycle defers without writing into the bare mount-point directory.
4. Restore ASIAIR and confirm a later cycle mounts it and resumes copying without
   a service restart.
5. Repeat the loss-and-restoration check for the NAS without inspecting,
   reconfiguring, or coupling the worker to `wg1`.
6. Boot with both remote resources absent, restore them independently, and
   confirm synchronization begins only once both exact mounts are available.

Unavailable resources are normal and produce a successful deferred cycle.
Incorrect mount identities are also deferred to protect local storage. Genuine
rclone failures remain visible as failed service runs; the timer continues to
schedule later attempts.

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
- removing the QNAP NFS permission for `192.168.4.9/32` caused the destination
  watchdog to stop the active cycle; and
- restoring that permission allowed the next five-minute timer cycle to resume
  full FITS copies without manual recovery;
- rebooting with ASIAIR unavailable and the NAS available did not block boot:
  both automount units returned active and waiting while their underlying mounts
  remained inactive; and
- after correcting the qualified source to `/mnt/asiair/ASIAIR/`, an installed
  dry run proposed `log/...` and `Plan/...` paths directly beneath the
  destination, and a live run copied multiple full FITS files as `Plan/...`
  without creating another intermediary `ASIAIR` directory.

An earlier pre-correction live run created
`/mnt/library/astro/anser/ASIAIR/`. It has deliberately not been moved or
deleted by the service. Allow the corrected additive copy to finish and verify
the direct destination before removing that obsolete tree as a separate,
explicit operator action.
