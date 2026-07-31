# Local Production Startup

This is the temporary production startup procedure until GRE-224 and GRE-226
complete independently supervised appliance services and qualified releases.
It runs the real ASI662MC, the release-mode Rust application, hardware H.264,
and the pinned MediaMTX binary. MediaMTX uses a dedicated network namespace
because Pion requires netlink while creating ICE peer connections. It can see
only namespace loopback and a private media veth. Exact-destination nftables
rules expose WHEP TCP/8889 and ICE UDP/8189 only at the two approved service
addresses. ObsCam remains a manual foreground process. This is not a
deterministic test stack, and GRE-224 still owns the final installer and managed
service integration.

## Prerequisites

- Run commands from the repository root.
- `/usr/local/bin/mediamtx` must report version `v1.19.3`.
- Its SHA-256 must match `deploy/mediamtx-linux-arm64.sha256`.
- `/etc/obscam/mediamtx.yml` is the installed host configuration. It matches
  the checked production media path while advertising only the permitted
  `wg0` and LAN addresses.
- `/etc/obscam/obscam-media.nft` and
  `/usr/local/libexec/obscam/setup-media-network` match the checked-in namespace
  boundary. IPv4 forwarding must already be enabled; the helper fails closed
  and never changes that host-global setting.
- `/etc/systemd/system/obscam-media-network.service` matches the checked-in
  one-shot namespace owner.
- `/etc/systemd/system/obscam-mediamtx.service` matches the checked-in temporary
  unit. MediaMTX receives private-veth RTP and enters `/run/netns/obscam-media`.
- The release binary must be built from the intended checkout.

Verify the installed relay:

```sh
mediamtx --version
sha256sum /usr/local/bin/mediamtx
sed -n '$p' deploy/mediamtx-linux-arm64.sha256
cmp --silent deploy/mediamtx.yml /etc/obscam/mediamtx.yml
cmp --silent deploy/obscam-media.nft /etc/obscam/obscam-media.nft
cmp --silent deploy/setup-media-network \
  /usr/local/libexec/obscam/setup-media-network
cmp --silent deploy/systemd/obscam-media-network.service \
  /etc/systemd/system/obscam-media-network.service
cmp --silent deploy/systemd/obscam-mediamtx.service \
  /etc/systemd/system/obscam-mediamtx.service
/usr/sbin/sysctl -n net.ipv4.ip_forward
```

Build the browser assets and production application after changing either:

```sh
npm test
cargo build --release -p obscam
```

## Start

Install the reviewed temporary boundary after either checked-in deployment file
changes:

Before the first install, stop if either destination already exists with
different content. Preserve a recoverable copy and obtain explicit operator
approval to adopt that path; these commands are only authorized for absent,
matching, or explicitly adopted destinations. GRE-224 must turn this manual
boundary into an ownership-enforcing installer.

```sh
sudo install -D -m 0644 deploy/mediamtx.yml /etc/obscam/mediamtx.yml
sudo install -D -m 0644 deploy/obscam-media.nft \
  /etc/obscam/obscam-media.nft
sudo install -D -m 0755 deploy/setup-media-network \
  /usr/local/libexec/obscam/setup-media-network
sudo install -D -m 0644 deploy/systemd/obscam-media-network.service \
  /etc/systemd/system/obscam-media-network.service
sudo install -D -m 0644 deploy/systemd/obscam-mediamtx.service \
  /etc/systemd/system/obscam-mediamtx.service
sudo systemctl daemon-reload
```

Use two terminals. Start MediaMTX first only for clearer logs; ObsCam does not
depend on startup ordering.

Terminal one runs:

```sh
sudo systemctl start obscam-mediamtx.service
sudo journalctl --follow --unit obscam-mediamtx.service
```

The installed configuration disables candidate advertisement from discovered
interfaces and advertises only `10.164.190.1` and `192.168.1.200`. The namespace
is the independent discovery boundary: MediaMTX can enumerate only `lo` and
`media0`, while nftables translates only the two approved destination addresses
and service ports. Forwarded namespace traffic is limited to established flows
and ICE from UDP/8189 to the approved WireGuard and LAN client networks; other
namespace forwarding and host access are dropped. No other host interface is
named or visible to MediaMTX.

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

Press `Ctrl-C` once in the ObsCam terminal, then stop the temporary relay unit:

```sh
sudo systemctl stop obscam-mediamtx.service obscam-media-network.service
```

GRE-224 owns the complete installer, final service identity decisions, reboot
and recovery qualification, and integration of this temporary MediaMTX unit
into the independently supervised appliance service set.
