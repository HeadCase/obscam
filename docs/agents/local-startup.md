# Managed Appliance Startup

GRE-224 installs ObsCam and pinned MediaMTX as independently supervised
appliance services. Until GRE-226 adds immutable release activation and
rollback, the installer consumes two explicit, locally available binaries and
refuses to replace differing destinations other than the known GRE-218
temporary-service predecessors.

The managed stack runs the real ASI662MC, release-mode Rust application,
hardware H.264, and MediaMTX v1.19.3. MediaMTX remains inside the dedicated
`obscam-media` network namespace. ObsCam neither requires nor follows MediaMTX;
camera capture, control, and local recovery start independently when the relay
is absent.

## Build and install

Run from the repository root. Build browser assets before the release binary:

```sh
npm test
cargo build --release -p obscam
```

Provide the exact MediaMTX v1.19.3 Linux ARM64 binary as an explicit installer
input. The checked checksum is `deploy/mediamtx-linux-arm64.sha256`; the
installer verifies its hash and reported version before changing the host.

```sh
sudo deploy/install-appliance install \
  --obscam-binary target/release/obscam \
  --mediamtx-binary /path/to/mediamtx
```

The installer:

- creates the non-login `obscam` identity and its exact camera-access group;
- installs both binaries under `/usr/local/libexec/obscam`;
- installs the pinned relay configuration, namespace/firewall boundary,
  checksum manifests, udev rules, and systemd units;
- restricts ASI662MC `03c3:662b` to `root:obscam-camera 0660` while Rust retains
  the factory-serial ownership check;
- assigns only the named `bcm2835-codec-encode` node to the `obscam` owner while
  preserving the host `video` group;
- enables all three local units without starting remote-resource dependencies;
- refuses unknown differing files or symlink destinations.

Repeat the read-only installed contract check at any time:

```sh
sudo deploy/install-appliance verify
systemd-analyze verify \
  obscam.service \
  obscam-media-network.service \
  obscam-mediamtx.service
```

GRE-226 will replace this binary-copy boundary with immutable compatibility
sets, a stable active-release link, and automatic rollback.

## Start and reboot

The units are enabled for `multi-user.target`; no manual ordering is required:

```sh
sudo systemctl start \
  obscam-media-network.service \
  obscam-mediamtx.service \
  obscam.service
```

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
/usr/local/libexec/obscam/verify-mediamtx \
  /usr/local/libexec/obscam/mediamtx \
  /etc/obscam/mediamtx.yml \
  /usr/local/share/obscam/mediamtx-linux-arm64.sha256 \
  /usr/local/share/obscam/mediamtx-config.sha256
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

Managed logs exist only in journald and are rate-limited per unit:

```sh
systemctl status --no-pager \
  obscam.service obscam-mediamtx.service obscam-media-network.service
sudo journalctl --unit obscam.service --since today
sudo journalctl --unit obscam-mediamtx.service --since today
sudo journalctl --unit obscam-media-network.service --since today
systemctl show obscam.service obscam-mediamtx.service \
  -p User -p Group -p DynamicUser -p NRestarts -p ActiveState \
  -p CPUUsageNSec -p MemoryCurrent -p MemoryPeak -p TasksCurrent
```

The following read-only checks cover the rest of the qualified deployment
signals without activating optional mounts or probing unrelated interfaces:

```sh
# Qualified binaries, configuration, and health
sha256sum /usr/local/libexec/obscam/obscam
/usr/local/libexec/obscam/verify-mediamtx \
  /usr/local/libexec/obscam/mediamtx \
  /etc/obscam/mediamtx.yml \
  /usr/local/share/obscam/mediamtx-linux-arm64.sha256 \
  /usr/local/share/obscam/mediamtx-config.sha256
curl --fail http://127.0.0.1:8080/api/v1/health

# Optional storage and the approved browser VPN only
systemctl status --no-pager mnt-asiair.automount mnt-library.automount
wg show wg0

# Host capacity and Raspberry Pi thermal/throttling state
systemctl show obscam.service obscam-mediamtx.service \
  -p CPUUsageNSec -p MemoryCurrent -p MemoryPeak -p TasksCurrent
free -h
vcgencmd measure_temp
vcgencmd get_throttled
cat /sys/class/thermal/thermal_zone0/temp
```

Do not substitute a deterministic camera, ambient MediaMTX binary, secondary
media path, or custom startup stack for production qualification.
