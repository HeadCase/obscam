# Qualified Appliance Releases

GRE-226 installs ObsCam as an immutable qualified compatibility set. The Rust
application and its embedded browser assets, MediaMTX v1.19.3, relay
configuration, systemd definitions, deployment helpers, and ZWO SDK 1.41 are
hashed together beneath `/opt/obscam/releases/<release-id>`. Stable
`active` and `previous` links switch the complete set; host configuration stays
separate at `/etc/obscam/host.env`.

The managed stack runs the real ASI662MC, release-mode Rust application,
hardware H.264, and MediaMTX v1.19.3. MediaMTX remains inside the dedicated
`obscam-media` network namespace. ObsCam neither requires nor follows MediaMTX;
camera capture, control, and local recovery start independently when the relay
is absent.

## Build and install

The live host must expose the cgroup v2 memory controller. The installer checks
`/sys/fs/cgroup/cgroup.controllers` before changing any live file and refuses
installation when `memory` is absent. On the qualified Raspberry Pi OS image,
append `cgroup_enable=memory cgroup_memory=1` to the existing single line in
`/boot/firmware/cmdline.txt`, reboot, and confirm `memory` is present before
installation. Do not replace the existing root, console, or device arguments.

Run from the repository root. Build browser assets before the release binary:

```sh
npm test
cargo build --release -p obscam
```

Provide the exact MediaMTX v1.19.3 Linux ARM64 binary and ZWO SDK 1.41 library
as explicit installer inputs. Their checked hashes are
`deploy/mediamtx-linux-arm64.sha256` and
`deploy/zwo-sdk-linux-arm64.sha256`. Copy and edit the host configuration
example only when the local bind or camera defaults need to differ.

```sh
cp deploy/host.env.example /tmp/obscam-host.env
sudo deploy/install-appliance install \
  --release-id gre-226-<commit> \
  --obscam-binary target/release/obscam \
  --mediamtx-binary /path/to/mediamtx \
  --zwo-sdk-library /usr/local/lib/libASICamera2.so.1.41 \
  --host-config /tmp/obscam-host.env
```

The first migration from the installed GRE-225 layout additionally requires
`--adopt-gre-225`. This explicit operation verifies the existing identities,
binaries, relay pin, configuration, and service paths, preserves the replaced
host definitions with a `.pre-gre-226` suffix, and refuses any unrecognized
predecessor. Do not use the option for a clean installation or later upgrade.

The installer:

- creates the non-login `obscam` identity and its exact camera-access group;
- validates and freezes the complete release under `/opt/obscam/releases`;
- installs stable systemd, udev, sysusers, diagnostics, and release-manager
  links through `/opt/obscam/active`;
- installs the read-only diagnostic command and the repository-owned AllSky
  resource-policy drop-in without replacing AllSky's service definition;
- restricts ASI662MC `03c3:662b` to `root:obscam-camera 0660` while Rust retains
  the factory-serial ownership check;
- assigns only the named `bcm2835-codec-encode` node to the `obscam` owner while
  preserving the host `video` group;
- enables `obscam.target` as the single boot and operator lifecycle unit;
- refuses unknown paths, identities, configuration, binaries, and symlink
  destinations;
- switches the complete set atomically, runs bounded production checks, and
  restores both prior release links if any check fails.

Repeat the read-only installed contract check at any time:

```sh
sudo /usr/local/sbin/obscam-release verify
/opt/obscam/active/bin/verify-memory-controller
systemd-analyze verify \
  obscam.target \
  obscam.service \
  obscam-media-network.service \
  obscam-mediamtx.service
```

Inspect or roll back without rebuilding a release:

```sh
/usr/local/sbin/obscam-release status
sudo /usr/local/sbin/obscam-release rollback
```

`status` is read-only and reports active, previous, and interrupted activation
state. `rollback` validates the previous set and host configuration, switches
the whole set, and applies the same post-activation gate. A mutating install or
rollback first recovers any activation interrupted after its atomic link
switch.

GRE-226 will replace this binary-copy boundary with immutable compatibility
sets, a stable active-release link, and automatic rollback.

## Start and reboot

`obscam.target` is enabled for `multi-user.target`; no manual ordering is
required:

```sh
sudo systemctl start obscam.target
sudo systemctl stop obscam.target
sudo systemctl restart obscam.target
```

The target is the normal operator interface. Its three members remain separate
services, so an unexpected ObsCam or MediaMTX exit still restarts only the
failed process. Direct per-service commands remain available for isolated
diagnostics and recovery.

At reboot, all three start from local readiness. They do not order themselves
after `network-online.target`, WireGuard, DNS, internet access, remote mounts,
or optional storage. ObsCam and MediaMTX do not order themselves after one
another. The namespace owner is the only local prerequisite of MediaMTX.

Expected camera and FFmpeg faults recover inside Rust. Unexpected ObsCam,
MediaMTX, or namespace-setup exits receive delayed systemd retries with no
start-limit lockout. No systemd watchdog is configured.

## Verify the running stack

```sh
systemctl is-active \
  obscam.target \
  obscam.service \
  obscam-mediamtx.service \
  obscam-media-network.service
curl --fail http://127.0.0.1:8080/api/v1/health
curl --fail http://127.0.0.1:8080/api/v1/runtime
sudo journalctl --unit obscam.service --unit obscam-mediamtx.service
```

The installed MediaMTX unit runs the private binary and refuses to start unless
its binary hash, `v1.19.3` version, configuration hash, and exactly one
MoQ-disabled setting all match:

```sh
systemctl show obscam-mediamtx.service -p ExecStartPre -p ExecStart
/opt/obscam/active/bin/verify-mediamtx \
  /opt/obscam/active/bin/mediamtx \
  /opt/obscam/active/config/mediamtx.yml \
  /opt/obscam/active/config/mediamtx-linux-arm64.sha256 \
  /opt/obscam/active/config/mediamtx-config.sha256
```

On the observatory LAN, open `http://192.168.1.200:8080`. Over the permitted
ObsCam WireGuard route, open `http://10.44.0.1:8080`. Never inspect or advertise
`wg1`; it is unrelated storage infrastructure.

MediaMTX should report path `obscam` online with one H.264 track. A browser WHEP
connection should return HTTP 201 and display native 1920×1080 video. Stopping
MediaMTX must leave capture, settings, control, and encoder ownership intact;
restarting it must restore relay readiness and a fresh decodable GOP without
restarting ObsCam.

## Diagnostics

Run the installed read-only diagnostic command as the operator. Individual
sections report failures and continue so an unavailable camera, VPN, or mount
does not hide the remaining host evidence:

```sh
/usr/local/bin/obscam-diagnostics
```

The report includes qualified binary/configuration evidence, service state,
the latest 100 journal records, health, optional mount state, the approved
`wg0` VPN, resource controls and usage, and thermal/throttling signals. It does
not start or restart services, activate mounts, or inspect unrelated network
interfaces. See [`docs/operations/shared-host.md`](../operations/shared-host.md)
for interpretation and the individual fallback commands.

Do not substitute a deterministic camera, ambient MediaMTX binary, secondary
media path, or custom startup stack for production qualification.
