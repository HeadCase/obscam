# GRE-180: Native ASI662MC capture ceiling and adapter opportunity

## Answer

ZWO publishes **1920 x 1080** and **107.6 frames/s in high-speed mode** for
the ASI662MC/MM. That is a marketed sensor/camera ceiling, not a demonstrated
full-stack ceiling on the deployed Raspberry Pi. ZWO does not state on the
product page which SDK output format, exposure, USB-bandwidth setting, host,
or dropped-frame tolerance produced 107.6 frames/s. The specification should
therefore treat **107.6 fps (9.29 ms/frame) as the capture ceiling to test, not
as an acceptance result**. [ZWO ASI662MC/MM product page][zwo-662]

A replacement for ZWO's native camera driver is **not warranted**. The official
SDK already provides ARMv7 and ARMv8 libraries and a small C ABI that writes
into caller-owned memory. Reimplementing the USB protocol would add substantial
compatibility and recovery risk without evidence of a higher sensor/USB
ceiling.

A new language binding by itself is also **unlikely to produce a dramatic
gain**. `python-zwoasi` uses `ctypes.CDLL`, lets the SDK write directly into a
caller-supplied `bytearray`, and exposes that memory as a NumPy view. Its
blocking foreign call releases the GIL. Its default convenience path does,
however, allocate a new full-frame buffer and query ROI state for every frame.
Those costs can be removed without changing languages. [python-zwoasi capture
source][py-capture] [Python `CDLL` GIL contract][python-cdll] [NumPy
`frombuffer` contract][numpy-frombuffer]

The worthwhile opportunity is narrower and more consequential: prototype a
**native capture-pipeline module** that owns the ZWO SDK lifecycle, a fixed
buffer pool, setting generations, frame timestamps/drop counters, and direct
handoff to the selected encoder. Compare it with a corrected, buffer-reusing
Python path. Adopt the native module only if measurement shows a material
end-to-end benefit or if direct encoder handoff removes a copy that the Python
path cannot. Do not choose C++, Rust, or a wholesale native server merely
because the camera SDK is native.

## Authoritative SDK contract

The root contract is ZWO's `ASICamera2.h` and `libASICamera2`, not
`python-zwoasi`. ZWO's current [Product SDK page][zwo-sdk-download] offered
ASI Camera SDK V1.41 during this research. The downloaded official package:

- contains `ASICamera2.h`, C++ video/snapshot demos, and proprietary
  `libASICamera2` binaries for ARMv6, ARMv7, ARMv8, x86/x64, and macOS;
- links the ARM libraries to `libusb-1.0` and exports the documented C ABI;
- does not contain the implementation source for `libASICamera2`;
- has package SHA-256
  `c6d59bf2de5b807f13d60d10b035e10dff31aeaa43391c6cd4782242126775ec`;
- has header SHA-256
  `af6ab82e66905b3a0f3313e1e82ccb4fedde272eaa64744588a1be8103f9cf0c`.

ZWO's public [ASICamera2 SDK manual][zwo-sdk-manual] is revision 2.9 dated
2023-01-11. It remains authoritative for behavior, but the current V1.41 header
must win where signatures differ. For example, that manual prints
`ASIStartExposure(int camera_id)`, while V1.41 declares
`ASIStartExposure(int camera_id, ASI_BOOL is_dark)`. A binding must be built and
tested against the deployed library version, log `ASIGetSDKVersion()` at
startup, and fail clearly on an unsupported ABI.

The SDK's intended fast path is:

1. set full-frame ROI and output format while capture is stopped;
2. call `ASIStartVideoCapture()` once;
3. repeatedly drain `ASIGetVideoData()` from a dedicated thread into a
   caller-allocated buffer;
4. stop capture before changing ROI/output format.

ZWO explicitly warns that the SDK video buffer is small and frames are
discarded unless `ASIGetVideoData()` is called quickly. The manual recommends
an asynchronous circular buffer; `ASIGetDroppedFrames()` reports losses. This
means capture draining must never wait for JPEG/video encoding or client I/O.
[ZWO SDK manual, video acquisition][zwo-sdk-manual]

## Published ceiling versus required measurement

At full frame, the public ABI requires these uncompressed caller buffers:

| SDK format | Bytes/frame | At 107.6 fps | Meaning |
| --- | ---: | ---: | --- |
| `Y8` | 2,073,600 | 223.1 MB/s | SDK monochrome output for a colour camera |
| `RAW8` | 2,073,600 | 223.1 MB/s | 8-bit sensor samples; Bayer mosaic on this colour camera |
| `RAW16` | 4,147,200 | 446.2 MB/s | 16-bit container |
| `RGB24` | 6,220,800 | 669.4 MB/s | three-byte colour output |

The buffer sizes are specified by ZWO. The interpretation of `RAW8` as Bayer
data is an inference from the camera's colour Bayer property/pattern plus the
separate colour-only `Y8` and `RGB24` modes; ZWO's manual does not spell out the
raw mosaic layout beyond the reported Bayer pattern. [ZWO SDK manual, image
types and buffer sizes][zwo-sdk-manual]

The 669.4 MB/s payload implied by full-frame `RGB24` at 107.6 fps exceeds the
625 MB/s signalling rate of nominal 5 Gbit/s USB 3 before protocol overhead.
Therefore the published maximum cannot be assumed for `RGB24`. This is a
calculation, not a ZWO claim. ZWO states only that the camera has USB 3.0 and a
256 MB DDR3 cache. [ZWO ASI662MC/MM product page][zwo-662]

On Raspberry Pi 4, the two USB 3 ports and two USB 2 ports connect through the
VL805 controller; the Pi's official documentation also describes the BCM2711
PCIe link feeding USB and documents H.264 encoding at 1080p30. Consequently,
USB acquisition, colour conversion, and encoding can each impose a different
ceiling. A camera adapter benchmark alone cannot establish browser-visible
performance. [Raspberry Pi USB topology][rpi-usb] [BCM2711 multimedia and I/O
specification][rpi-bcm2711]

The deployed system must measure, rather than infer:

- sustained unique-frame cadence and inter-frame p50/p95/p99 for `Y8`, `RAW8`,
  and supported colour outputs at 1920 x 1080, bin 1;
- exposure values spanning the operating range, especially 10 ms, 25 ms,
  50 ms, 100 ms, 200 ms, and representative long exposures;
- `ASI_HIGH_SPEED_MODE` and `ASI_BANDWIDTHOVERLOAD` supported ranges and their
  effect on cadence, corrupt frames, USB errors, and SDK dropped frames;
- warm-up behavior and the exact relation between exposure duration, rolling
  readout, and frame period;
- capture-only CPU, memory bandwidth, allocation rate, temperatures, and USB
  throughput, then the same metrics with colour conversion and encoding;
- results both on an otherwise quiet Pi and under the required Allsky,
  WireGuard, and `rclone` production load.

No hardware was available in this research worktree, so none of those values
has been measured here.

## B&W and colour semantics

The ASI662MC is a colour Bayer camera. B&W mode does not change the sensor into
a monochrome sensor:

- ZWO defines `Y8` as a colour-camera-only **monochrome mode**, one byte per
  pixel.
- ZWO defines `RGB24` as colour-camera-only, three bytes per pixel.
- `RAW8` is also one byte per pixel and preserves the information needed for
  Bayer colour reconstruction.
- The SDK does not document the algorithm used to derive `Y8`; its tonal
  response and cost must be characterized with the actual camera/SDK.
  [ZWO SDK manual, `ASI_IMG_TYPE`][zwo-sdk-manual]

Thus B&W is **3x lighter at SDK output than `RGB24`, but not lighter at USB
acquisition than `RAW8`**. The strongest likely pipeline is:

- B&W: acquire `Y8`, skip debayering, then encode grayscale;
- colour: acquire `RAW8`, perform one optimized debayer/colour conversion,
  then encode colour;
- benchmark that colour path against SDK `RGB24`, because `RGB24` may trade
  host CPU for three times the transfer volume.

This design can make B&W materially faster and cheaper downstream while both
modes retain full resolution. The size advantage after lossy compression is
scene- and encoder-dependent and must be measured; it is not guaranteed by the
SDK buffer-size ratio.

## Exposure interruption and truthful settings

The SDK supplies two acquisition regimes:

- **Video:** continuous frames via `ASIStartVideoCapture()` and
  `ASIGetVideoData()`.
- **Snapshot:** `ASIStartExposure()`, polling, and
  `ASIGetDataAfterExp()`. `ASIStopExposure()` explicitly cancels a long
  snapshot; ZWO notes that if the status is already successful, its image may
  still be read.

ZWO says control values can generally be set during capture, except exposure
in trigger mode, but it does not guarantee which returned frame first contains
a new gain/exposure. Nor does it specify whether stopping video concurrently
with a blocking `ASIGetVideoData()` is a supported cross-thread interrupt.
[ZWO SDK manual, controls and exposure][zwo-sdk-manual]

To implement the approved "abort, discard, apply, restart" behavior, the
capture owner should version every requested setting set. For snapshot mode it
should call `ASIStopExposure()` immediately and never publish the old
generation. For video mode it should stop, apply controls, restart, and discard
warm-up/old-generation frames. The benchmark must establish bounded abort
latency, whether cross-thread stop is safe, how many frames require discard,
and whether a short `ASIGetVideoData()` timeout is preferable to concurrent
SDK calls. Do not label a frame with requested settings merely because the
request was accepted.

## `python-zwoasi` cost profile

This repository pins `zwoasi==0.2.0`, which is the project's
[`v0.2.0` source][py-source]. Its path has useful properties:

- `_get_video_data()` accepts a caller-provided `bytearray`; `ctypes.from_buffer`
  points the C call at that memory, so there is no wrapper staging copy.
- `np.frombuffer()` creates a view rather than copying the completed buffer.
- `ctypes.cdll.LoadLibrary()` creates a `CDLL`; CPython releases the GIL while
  exported functions such as blocking `ASIGetVideoData()` run.

The default convenience call still creates a new 2.07 MB `bytearray` for every
full-frame `Y8`/`RAW8` frame, queries ROI to determine its size, calls the SDK,
then queries ROI again to shape the NumPy view. The wrapper already exposes the
buffer-reuse hook, so an optimized Python comparison must preallocate and cache
immutable format metadata before judging Python or FFI overhead.
[python-zwoasi buffer path][py-capture] [python-zwoasi NumPy path][py-numpy]

An FFI call cost exists, but the sources do not quantify it. It must be
isolated experimentally. At a 9.29 ms advertised frame period it may be
irrelevant compared with USB transfer and encoding; per-frame allocations,
format/control chatter, or serial processing may instead dominate.

## Evidence from the current repository

The present implementation is useful as a failure/control case, not as a
greenfield constraint:

- [`ZwoAsiCamera`](../src/obscam/camera/zwo_asi_camera.py) selects `Y8`, uses
  video mode at exposures up to 200 ms, then encodes every captured frame to
  JPEG with Pillow.
- It calls `_prepare_capture()` for every frame, including gain/exposure writes
  and `set_image_type()`. `python-zwoasi.set_image_type()` is implemented as a
  get-ROI/set-ROI cycle, while ZWO says ROI/format must only change while
  capture is stopped. This is invalid hot-path behavior once video capture is
  already running.
- It does not supply `capture_video_frame()` with a reusable buffer, so the
  wrapper allocates per frame.
- [`ContinuousCaptureLoop`](../src/obscam/core/capture_loop.py) checks queued
  settings only between blocking captures. A setting request during a long
  exposure therefore does not trigger the required immediate cancellation.
- The existing benchmark helpers do not provide a native C reference, use the
  wrapper's default allocating path, and include ROI/binning cases that are now
  outside the approved full-resolution requirement.

These issues are sufficient to reject current benchmark numbers as evidence
of the camera's ceiling. They are not proof that Python cannot reach it.

## Adapter alternatives

| Alternative | Expected value | Main risk | Verdict |
| --- | --- | --- | --- |
| Corrected `python-zwoasi` hot path | Reuses fixed buffers, caches format, keeps control writes off the frame loop; minimal build burden | Python-side encode/handoff can still add copies and scheduling jitter | Required comparison baseline |
| Thin C/C++/Rust binding only | Removes Python FFI dispatch | Still calls the same proprietary SDK and public buffer-copy ABI | Do not build without measured wrapper loss |
| Native capture-pipeline module | Fixed ring, latest-frame dropping, direct encoder handoff, generation metadata, isolated failure boundary | ABI packaging, process recovery, buffer ownership complexity | Recommended prototype after native benchmark harness |
| Fully native application/server | Maximum integration freedom | Large rewrite; transport/UI/control concerns become coupled to camera code | Not justified by camera evidence |
| Replacement USB driver | Could bypass undocumented SDK internals | Reverse engineering, firmware/USB compatibility, dual-camera and recovery risk | Reject |

C or C++ is the shortest path to a trustworthy reference because ZWO ships the
header, libraries, and C++ demos. Rust can wrap the same ABI and improve
ownership modeling, but every SDK call and caller-writable image buffer remains
an `unsafe` boundary; Rust has no inherent capture-speed advantage. Language
selection should follow the eventual encoder/transport integration, not lead
it.

## Required benchmark and decision gate

Before choosing the production adapter, build a throwaway native reference
harness around V1.41 (or the exact deployed SDK if different) and compare three
paths on the deployed Pi:

1. native C/C++ `ASIGetVideoData()` into a fixed 3-4 slot buffer ring, no
   processing;
2. `python-zwoasi` into an equivalent reused buffer ring, no processing;
3. each path with identical B&W or colour conversion and identical encoder.

For every test, record unique frames, SDK drops, corrupt/partial frames,
capture completion timestamp, inter-frame p50/p95/p99/max, CPU per process,
RSS, allocation rate, USB throughput/errors, temperature/throttling, and
setting-to-first-new-frame latency. Run long enough to expose thermal and
contention effects and repeat under the required production service load.

Adopt a native module when it removes a proven bottleneck: sustained capture
loss, unacceptable tail latency, dropped frames, excess CPU/memory churn, or a
copy between capture and encoder. If the corrected Python path is statistically
indistinguishable at the SDK boundary, retain Python for orchestration and put
optimization effort into debayering, encoding, fan-out, and browser delivery.

## Uncertainties carried forward

- ZWO's 107.6 fps claim does not identify image type, exposure, high-speed and
  USB-bandwidth control values, host, or drop criteria.
- The deployed `libASICamera2` version and bitness were not visible from this
  isolated worktree; they must be recorded on the Pi.
- `ASI662MC.SupportedVideoFormat` and all control capabilities/ranges must be
  queried from the real unit rather than assumed from the SDK-wide enum.
- ZWO does not document how `Y8` is derived or its exact latency relative to
  `RAW8`.
- The SDK does not define frame-setting metadata or cross-thread stop semantics
  tightly enough to guarantee interruption behavior without hardware tests.
- No ASI662MC or production Pi hardware was available here, so this ticket
  resolves the research question and measurement design, not the deployed
  performance baseline.

## Primary sources

- [ZWO ASI662MC/MM official product page][zwo-662]
- [ZWO ASICamera2 SDK manual, revision 2.9][zwo-sdk-manual]
- [ZWO official Product SDK download page][zwo-sdk-download]
- [python-zwoasi v0.2.0 source][py-source]
- [CPython `ctypes.CDLL` documentation][python-cdll]
- [NumPy `frombuffer` documentation][numpy-frombuffer]
- [Raspberry Pi official USB-bus documentation][rpi-usb]
- [Raspberry Pi official BCM2711 documentation][rpi-bcm2711]

[zwo-662]: https://www.zwoastro.com/product/asi662mc/
[zwo-sdk-manual]: https://zwoastro.yuque.com/olyczd/sfwyw6/kpde2odaw3h4ekix
[zwo-sdk-download]: https://www.zwoastro.com/software/product-sdk/
[py-source]: https://github.com/python-zwoasi/python-zwoasi/tree/007bbd2e52b67737274b420ea52cf59ccd962d2b
[py-capture]: https://github.com/python-zwoasi/python-zwoasi/blob/007bbd2e52b67737274b420ea52cf59ccd962d2b/zwoasi/__init__.py#L182-L202
[py-numpy]: https://github.com/python-zwoasi/python-zwoasi/blob/007bbd2e52b67737274b420ea52cf59ccd962d2b/zwoasi/__init__.py#L644-L664
[python-cdll]: https://docs.python.org/3/library/ctypes.html#ctypes.CDLL
[numpy-frombuffer]: https://numpy.org/doc/stable/reference/generated/numpy.frombuffer.html
[rpi-usb]: https://github.com/raspberrypi/documentation/blob/b245ee62eac7d28604a54d1a3bdb5b1f644dde8c/documentation/asciidoc/computers/raspberry-pi/usb-bus-on-raspberry-pi.adoc#L32-L36
[rpi-bcm2711]: https://github.com/raspberrypi/documentation/blob/b245ee62eac7d28604a54d1a3bdb5b1f644dde8c/documentation/asciidoc/computers/processors/bcm2711.adoc#L3-L17
