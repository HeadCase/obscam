# GRE-183 native full-resolution capture evidence

## Status

**Incomplete: hardware topology and disconnect blocker.** The deployed
ASI662MC and ASI178MC were both negotiated on the Raspberry Pi USB 2 tree behind
nested hubs. A diagnostic control sweep was stopped after both cameras
disconnected. No USB power-cycle, service stop, or topology change was attempted.

This branch contains valid smoke and partial USB 2 measurements. It does **not**
establish the ASI662MC USB 3 ceiling, sustained reliability, setting interruption
latency, or a final Python/C/Rust adapter decision.

## Compared paths

- Native C reference: one guarded caller-owned buffer, SDK calls directly from
  the capture loop, and full-buffer zlib CRC32.
- Corrected python-zwoasi: one reused `bytearray`, direct `get_video_data()` with
  no per-frame ROI query, NumPy view, or full-frame buffer allocation, followed
  by the same zlib CRC32.
- Rust pipeline: four guarded buffers, a dedicated capture thread, a one-frame
  latest slot that replaces unconsumed work, and an in-place zlib CRC32 consumer.

All paths use SDK video acquisition at native 1920x1080, report exact completion
and unique-frame intervals, and emit the same validated JSON schema. The host
monitor samples process CPU/RSS, temperature, throttling, USB topology, recent
USB kernel messages, and relevant shared-host processes without writing during
the timed cell.

## Binding environment

- Raspberry Pi 4, aarch64, four Cortex-A72 CPUs, 4 GB RAM
- ZWO SDK `1, 38, 0, 0`
- ASI662MC native frame: 1920x1080
- Advertised outputs: RAW8, RGB24, Y8
- `ASI_BANDWIDTHOVERLOAD`: 40-100, default 50
- `ASI_HIGH_SPEED_MODE`: 0-1, default 0
- Allsky, rclone, and WireGuard activity remained present
- No thermal throttling was reported; observed peaks were below 60 C

Live `lsusb -t` placed both cameras at 480 Mbit/s through a VIA hub and two
ASMedia high-speed hubs. The Pi's 5 Gbit/s root had no camera attached. The
ASI662MC was device `03c3:662b`; Allsky's ASI178MC was `03c3:178a`.

## Completed results

The 10-second cells below requested 10 ms exposures with high-speed mode off.
All returned frames were unique. Every completed cell reported zero SDK drops,
capture errors, guard corruption, and Rust latest-slot drops.

| Format | USB control | Native C fps | Python fps | Rust fps |
| --- | ---: | ---: | ---: | ---: |
| Y8 | 40 | 8.357 | 8.343 | 8.348 |
| Y8 | 50 | 10.434 | 10.429 | 10.421 |
| RAW8 | 50 | 10.429 | 10.418 | 10.433 |
| RGB24 | 50 | 10.430 | 10.415 | 10.423 |

At the default USB control, path cadence differed by less than 0.2%. The
corrected Python binding was therefore not a measurable acquisition bottleneck
under this USB 2 ceiling. The Rust pipeline also had no opportunity to overlap
meaningful downstream work because capture itself took roughly 96 ms per frame.

One Python Y8 cell at USB control 100 completed before the failure. It achieved
19.277 unique fps with capture-call p50/p95/p99/max of
44.824/47.042/52.947/63.759 ms and zero reported SDK drops or capture errors.
The next runner could not find the ASI662MC. Kernel records then show:

- `00:25:39`: ASI178MC USB disconnect
- `00:25:49`: ASI662MC USB disconnect

Neither camera remained in `lsusb`, so the matrix stopped. Earlier logs also
show regular ASI178MC resets and both camera devices being reset between runner
processes. This is evidence of a shared USB/hub recovery problem, not evidence
that the successfully completed bandwidth-100 cell was itself corrupt.

## Resource signal

The 10-second default-control runs observed approximate peak RSS ranges of:

- Native C: 20.4 MB for Y8/RAW8 and 24.6 MB for RGB24
- Python: 63.5 MB for Y8/RAW8 and 67.6 MB for RGB24
- Rust: 27.0 MB for Y8/RAW8 and 43.6 MB for RGB24

The Rust values include four complete guarded frame buffers; Python and C each
use one frame buffer. Process CPU samples suggest Python has additional CRC/FFI
overhead, but the partial run and low capture ceiling are insufficient for an
adapter decision.

## Required continuation

1. Restore both cameras and connect the ASI662MC so SDK discovery reports USB 3
   host negotiation; record the resulting live topology.
2. Confirm the ASI178MC/Allsky path remains stable while the ASI662MC is tested.
   The resumed harness must retain its non-opening model selection and 100 ms
   ASI178MC sysfs sentinel; a production-camera disappearance aborts the cell.
3. Resume a short bandwidth/high-speed sweep first. Do not proceed to the full
   matrix if either camera resets or disappears.
4. Complete representative exposures, repeated finalist cells, setting-change
   interruption/classification, disconnect/reconnect recovery, a 30-minute
   finalist run, and the two-hour shared-load soak.
5. Only then decide whether corrected Python is indistinguishable, or whether
   the Rust latest-frame pipeline removes proven CPU, copy, tail-latency, or
   recovery costs.

Raw artifacts are under [`results/`](results/). They include the complete common
result, resource samples, and host snapshots for every successful cell.
