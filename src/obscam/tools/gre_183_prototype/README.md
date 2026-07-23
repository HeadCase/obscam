# GRE-183 native capture prototype

**Throwaway evidence tooling:** this directory answers whether corrected Python,
a minimal native reference, or a production-shaped Rust capture pipeline changes
full-frame ASI662MC acquisition behavior on the deployed Raspberry Pi 4. It is
not a production camera backend.

All runners use full 1920x1080 frames, caller-owned buffers, SDK video capture,
and full-buffer CRC32 as identical downstream work. They emit the versioned JSON
contract in `contract.py`. Exact duplicates are reported separately from strict
corruption signals.

## Commands

Build the native runners without opening a camera:

```bash
python -m obscam.tools.gre_183_prototype build
```

Discover the binding camera and its controls:

```bash
python -m obscam.tools.gre_183_prototype discover
```

Run a short baseline across all supported formats and runners:

```bash
python -m obscam.tools.gre_183_prototype smoke --output /tmp/gre-183-smoke
```

Run the screening matrix only after discovery reports USB 3 host negotiation:

```bash
python -m obscam.tools.gre_183_prototype screen \
  --output research/gre-183-native-capture/results/usb3-screen
```

`screen` refuses a USB 2 camera unless `--allow-usb2` is passed explicitly.
Each completed cell is written atomically and reused when the same output
directory is resumed. Camera-not-found failures receive two bounded retries to
capture transient USB re-enumeration without hiding persistent disconnects.

The C runner needs the official `ASICamera2.h`; the default matches the deployed
SDK 1.38 installation. The Rust runner has no crate dependencies and links the
same SDK and system zlib libraries directly.

## Safety boundary

The benchmark does not stop Allsky, rclone, or WireGuard and does not power-cycle
USB hubs. A camera or hub disconnect stops the matrix. Quiet-baseline service
changes and physical USB topology changes require operator coordination.
