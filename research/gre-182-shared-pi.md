# Shared-Pi coexistence constraints (GRE-182)

**Research date:** 2026-07-23
**Target:** the deployed Raspberry Pi 4 (4 GB), concurrently running ObsCam, Allsky,
a WireGuard server, a WireGuard client, and rclone.

## Executive answer

Running ObsCam and Allsky as separate native processes, each owning one ZWO camera,
is a credible design, but it is not yet a proved deployment property. The native ZWO
SDK explicitly supports operating multiple cameras and gives each connected camera a
runtime `CameraID`; its public contract does not state that separate processes, or
processes embedding different SDK versions, can safely operate different cameras.
That gap requires a hardware fault-and-soak test before the architecture is accepted.

The Pi has no dedicated USB controller per socket. On Pi 4, all four external sockets
are behind one VL805 controller, and every USB 2 device shares one USB 2 hub. Camera
placement, negotiated speed, simultaneous traffic, power, and temperature are
therefore more important than nominal port colour. Allsky also has bursty phases that
an idle or daytime benchmark will miss.

The safest initial resource policy is to leave both camera services and the VPN
dataplane unconstrained, then make rclone explicitly yield through its own bandwidth,
concurrency, memory, CPU, and I/O controls. Kernel WireGuard traffic is not protected
by increasing the resource weight of the short-lived `wg-quick` configuration service.
Hard CPU quotas, CPU pinning, and real-time scheduling should not be introduced until
measurements identify a specific scheduler failure.

## Decision implications

1. **Use one owner per camera.** ObsCam should open the ASI662MC once in one native
   capture process. Allsky remains the sole owner of its camera. ObsCam must never
   pause, reconfigure, or recover Allsky.
2. **Bind by verified physical identity, not enumeration ordinal.** At startup,
   enumerate all devices, read their native properties, match the configured identity,
   and only then retain the returned runtime `CameraID`. Prefer a serial number if the
   installed SDK proves it is available; otherwise provision and validate a unique
   flash `ASI_ID`, or use a documented physical USB path plus model as the binding.
3. **Treat native separate-process operation as an acceptance test, not an SDK fact.**
   Test cold boots, every start order, restarts, and every unplug/replug permutation.
   A wrong-camera open or cross-camera control is a release blocker.
4. **Require USB 3 negotiation for the performance camera.** Connect both cameras
   directly where possible and verify the live topology with `lsusb -t`. Different
   blue sockets do not provide independent host controllers.
5. **Throttle background transfer first.** Start rclone with one transfer, few
   checkers, limited or disabled multi-thread streams, capped buffer memory, and a
   measured `--bwlimit`. Give it a low CPU weight/nice level and, only where the
   deployed block scheduler supports it, a low I/O weight.
6. **Benchmark the real Allsky lifecycle.** The acceptance run must include its actual
   night capture configuration, per-frame modules/uploads, mini-timelapses, and its
   dawn/end-of-night keogram, startrail, and software-H.264 work.
7. **Hardware H.264 remains a promising ObsCam path, not an established capacity.**
   Raspberry Pi documents hardware H.264 on Pi 4, but not zero-copy ingestion or the
   sustainable rate for ZWO-owned buffers. Measure copies, encoder throughput,
   temperature, and viewer latency in the delivery prototype.
8. **Escalate hardware only after a controlled failure.** First establish adequate
   power/cooling, correct USB 3 operation, rclone yielding, and a stable pipeline. An
   upgrade exception is justified only if the agreed camera/VPN acceptance criteria
   still fail under that baseline.

## Verified facts

### Native ZWO camera contract

- The official SDK sequence is: count connected cameras, call
  `ASIGetCameraProperty()` for each index, then use the returned `CameraID` with
  `ASIOpenCamera()` and `ASIInitCamera()`. The documentation says refreshing devices
  does not change that returned ID and explicitly says the SDK can operate multiple
  cameras, distinguished by `CameraID`.
- `ASI_ID` is a separate, mutable identifier stored in camera flash and is documented
  for USB 3 cameras. It must not be confused with enumeration index or runtime
  `CameraID`.
- The recommended video path separates frame retrieval/saving into a thread;
  `ASIGetVideoData()` waits internally rather than consuming CPU in a spin loop.
- The public SDK document does **not** specify cross-process concurrency, library
  version interoperability, or two processes each owning a different camera.

Source: [ZWO ASICamera2 Software Development Kit, revision 2.9
(official)](https://zwoastro.yuque.com/olyczd/sfwyw6/kpde2odaw3h4ekix).

### Allsky's camera binding and workload

- Allsky passes the configured `cameranumber` to `capture_ZWO`; the current native
  path calls `ASIOpenCamera(CG.cameraNumber)` and
  `ASIGetCameraProperty(..., CG.cameraNumber)`. Its own change script warns that
  `cameranumber` can change when a camera is removed. The current selection path is
  therefore vulnerable to enumeration/order changes even though Allsky can report a
  camera's flash ID.
- Allsky links an architecture-specific static `libASICamera2.a` into its ZWO capture
  binary. This avoids same-process global state with ObsCam, but it does not establish
  USB-device coexistence and permits SDK version skew between binaries.
- Its save thread encodes/writes the image with OpenCV and invokes `saveImage.sh` in
  the background. If that save thread is still occupied when a new image is ready,
  Allsky skips the new save and logs a warning.
- `saveImage.sh` invokes Python `flow-runner.py`; the source says most per-image
  post-processing time is spent there. The project supports per-image processing,
  copies/uploads, thumbnails and mini-timelapses.
- End-of-night processing can create a keogram, startrails, and a timelapse and perform
  uploads. Background end-of-night work is run at nice level 15. The default timelapse
  codec in the current configuration template is software `libx264`.

Sources: immutable Allsky commit `a7b32445b9689969af3c757e4888a2db90e1a86e`:
[camera-number warning](https://github.com/AllskyTeam/allsky/blob/a7b32445b9689969af3c757e4888a2db90e1a86e/scripts/makeChanges.sh#L323-L330),
[capture open/property calls](https://github.com/AllskyTeam/allsky/blob/a7b32445b9689969af3c757e4888a2db90e1a86e/src/capture_ZWO.cpp#L857-L874),
[capture save thread](https://github.com/AllskyTeam/allsky/blob/a7b32445b9689969af3c757e4888a2db90e1a86e/src/capture_ZWO.cpp#L144-L215),
[SDK linkage](https://github.com/AllskyTeam/allsky/blob/a7b32445b9689969af3c757e4888a2db90e1a86e/src/Makefile),
[per-frame flow runner](https://github.com/AllskyTeam/allsky/blob/a7b32445b9689969af3c757e4888a2db90e1a86e/scripts/saveImage.sh#L264-L267),
[end-of-night priority](https://github.com/AllskyTeam/allsky/blob/a7b32445b9689969af3c757e4888a2db90e1a86e/scripts/endOfNight.sh#L33-L41), and
[default timelapse codec](https://github.com/AllskyTeam/allsky/blob/a7b32445b9689969af3c757e4888a2db90e1a86e/config_repo/options.json.repo#L1527-L1538).

### Pi 4 USB, power, thermals, and codec

- Raspberry Pi 4 exposes two USB 3 and two USB 2 ports, but they are connected to one
  VL805 controller. The USB 2 lines from all four ports connect through a single USB 2
  hub, so USB 1.1/2 devices share one USB 2 port's total bandwidth.
- Multiple high-demand USB devices can contribute to undervoltage. USB peripheral
  power is additional to the board's own load; the official Pi 4 supply rating is
  5 V / 3 A.
- Pi 4 progressively throttles between 80 and 85 degrees Celsius, with Arm and GPU
  throttling at 85 degrees. Raspberry Pi documents `vcgencmd measure_temp` for the
  sensor and firmware clock/throttling telemetry; adequate cooling improves sustained
  performance.
- Raspberry Pi's camera tools use hardware H.264 where available. The documentation
  does not provide a capacity guarantee for frames arriving from a third-party USB
  camera or prove a zero-copy ZWO-to-encoder route.
- ZWO specifies the ASI662MC as 1920 x 1080, USB 3, with a maximum advertised frame
  rate of 107.6 fps. At RAW8 that image geometry and rate imply about 223 MB/s of image
  payload before USB/protocol overhead; this is arithmetic from published figures,
  not a measured bus rate.

Sources: [Raspberry Pi USB
documentation](https://www.raspberrypi.com/documentation/computers/raspberry-pi.html#universal-serial-bus-usb),
[frequency and thermal
control](https://www.raspberrypi.com/documentation/computers/raspberry-pi.html#frequency-management-and-thermal-control),
[Raspberry Pi camera video
documentation](https://www.raspberrypi.com/documentation/computers/camera_software.html#rpicam-vid), and
[official ZWO ASI662MC specifications](https://www.zwoastro.com/product/asi662mc/).

### WireGuard, rclone, and Linux resource controls

- Linux WireGuard creates a kernel network interface. Its protocol encrypts and
  authenticates packets and transports them over UDP. Consequently, active tunnel
  traffic consumes kernel/softirq CPU after the `wg-quick` setup process has exited.
- rclone exposes direct controls for transfer bandwidth (`--bwlimit`), parallel
  transfers (`--transfers`, default 4), parallel checking (`--checkers`, default 8),
  multi-thread transfer streams, and per-transfer/aggregate buffering.
- In cgroup v2, `cpu.weight` distributes available CPU proportionally under contention;
  it is work-conserving rather than a reservation. `io.max` supplies hard BPS/IOPS
  limits, while I/O weight effectiveness depends on the device and scheduler.
- systemd describes `MemoryHigh` as the primary mechanism for controlling memory use;
  exceeding it causes reclaim/throttling. `MemoryMax` is a last line of defence that
  can result in OOM handling.

Sources: [WireGuard protocol](https://www.wireguard.com/protocol/),
[WireGuard quick start](https://www.wireguard.com/quickstart/),
[rclone global options](https://rclone.org/docs/),
[Linux cgroup v2](https://docs.kernel.org/admin-guide/cgroup-v2.html), and
[systemd resource-control reference](https://github.com/systemd/systemd/blob/main/man/systemd.resource-control.xml).

## Inferences to validate

The following are design inferences, not guarantees from the cited projects:

- Separate processes should isolate application crashes and SDK global state, but can
  still conflict in libusb, kernel USB, or vendor device handling. Static linking in
  Allsky makes mixed native SDK versions a particular variable to record.
- If either camera negotiates at USB 2, simultaneous camera operation is unlikely to
  meet the ASI662MC's highest-rate modes because all USB 2 traffic shares 480 Mbit/s
  before protocol overhead. Even two USB 3 cameras still share the VL805 host path.
- A `CPUWeight` setting on `wg-quick.service` cannot reserve CPU for WireGuard's kernel
  dataplane. VPN reliability is better protected by retaining system/softirq headroom
  and constraining optional producers, chiefly rclone.
- Low rclone CPU and I/O weights are preferable to hard quotas at first: weights allow
  full idle capacity while yielding under contention. Application-level bandwidth and
  concurrency limits are more predictable than scheduler weights alone.
- CPU pinning can reduce scheduler flexibility on a four-core Pi, and real-time work
  can starve essential kernel or network processing. Neither should be a baseline
  configuration.
- Allsky's dawn batch may be a worse coexistence case than steady night capture due to
  simultaneous software encoding, image processing, storage, and upload activity.

## Deployment measurement plan

### 1. Inventory the actual host

Record the exact Pi revision, power supply, cooling, firmware, OS, kernel, cgroup mode,
storage device/mount/scheduler, NIC link, and WireGuard routes. Capture `lsusb -t` with
both cameras active and record each model, serial/flash ID if available, physical port,
negotiated speed, runtime `CameraID`, and driver path. Record the SDK version reported
by each binary, the Allsky commit/build and complete deployed camera/module/upload
configuration, and the rclone backend and command/service settings.

### 2. Prove identity and recovery

Run this matrix while asserting that each process opens only its assigned camera and
that controls never affect the other camera:

| Case | Required observation |
|---|---|
| Cold boot with both cameras | Stable identity mapping; both services healthy |
| Start Allsky first / ObsCam first / simultaneous start | Same mapping and behavior |
| Restart each service independently | Other capture remains uninterrupted |
| Unplug/replug each camera independently | No wrong-camera recovery or cross-control |
| Unplug both; reconnect in both possible orders | Identity is not based on enumeration order |
| Repeated timeout/disconnect cycles | Bounded recovery; no stale-frame masquerading |

Retain SDK error codes, USB resets, kernel logs, service logs, Allsky capture/save
warnings, and timestamps for every transition.

### 3. Exercise the real workload envelope

Measure ObsCam colour and B&W at full native spatial resolution, including roughly
10 ms exposure and representative longer exposures. For each mode, run:

- Allsky steady day and night capture with the deployed settings;
- deployed per-frame processing, copying, and uploading;
- mini-timelapse and dawn/end-of-night keogram, startrail, and timelapse phases;
- rclone scanning/checking and sustained transfer;
- both WireGuard roles active and four simultaneous viewers; and
- the combined worst realistic case.

Use at least a four-hour high-rate soak, plus a complete dawn/end-of-night phase and
the disconnect/recovery matrix. A full 24-hour operational cycle is preferable before
production acceptance.

### 4. Collect decision-grade metrics

- Acquisition: unique frames/s, dropped/duplicate/stale frames, SDK timeouts/errors,
  capture and encode queue depth.
- Viewer experience: exposure-end-to-display latency p50/p95/p99/max and honest
  client-visible frame rate for every viewer.
- Compute: per-process/thread CPU, system and softirq CPU, load, CPU/memory/I/O PSI,
  RSS, page faults, swap, and hardware encoder utilization/clock where exposed.
- I/O and network: block throughput/latency, NIC and each tunnel's throughput, loss and
  RTT, rclone progress, USB throughput/speed/errors/resets.
- Hardware health: temperature, Arm/GPU/H.264 clocks, throttling and undervoltage
  flags, and power-related kernel events.
- Co-tenant health: Allsky acquisition/save error rate and cadence versus its isolated
  baseline; VPN reachability/latency versus baseline.

Quantitative frame-rate and latency thresholds should come from the performance
contract rather than being invented here. This ticket supplies the coexistence gates:

1. zero wrong-camera opens or cross-camera control;
2. no VPN interruption attributable to ObsCam;
3. no material increase in Allsky capture/control failures versus baseline;
4. no USB reset, undervoltage, or thermal throttling in the accepted load envelope;
5. rclone remains best-effort and makes progress when higher-priority work permits;
6. disconnects do not leave apparently live stale video.

## Safe initial resource policy

1. Establish cooling, power, and verified USB 3 operation before tuning software.
2. Leave Allsky, ObsCam acquisition, and kernel networking without hard CPU quotas.
3. Configure rclone initially with `--transfers 1`, `--checkers 1` or `2`, constrained
   or disabled multi-thread streams, and bounded buffer memory. Derive `--bwlimit`
   from bandwidth remaining after the four-viewer acceptance run; no defensible fixed
   number exists without deployed stream and uplink measurements.
4. Run rclone with a low cgroup CPU weight (or idle weight where supported) and a high
   nice value. Apply low I/O weight or idle I/O scheduling only after confirming that
   the deployed block scheduler honors it. Consider `MemoryHigh`; keep `MemoryMax` only
   as a tested last-resort guard.
5. Observe system/softirq CPU and tunnel health directly. Do not treat the priority of
   `wg-quick.service` as WireGuard dataplane protection.
6. Introduce traffic shaping, CPU affinity, or encoder isolation only as separately
   measured responses to an identified bottleneck.

## Remaining unknowns

- Exact model, operating mode, USB speed, and bandwidth of the Allsky ZWO camera.
- Native SDK versions embedded in deployed Allsky and selected for ObsCam, and whether
  both versions support a usable hardware serial-number API.
- Whether two independently linked SDK instances can survive the full concurrent
  open, stream, error, and re-enumeration matrix on this host.
- Actual USB topology under the deployed hubs/cables and whether either camera falls
  back to USB 2 under load or after recovery.
- Deployed Allsky module schedule and the measured cost of its heaviest phase.
- Cooling, power headroom, storage scheduler, tunnel/uplink capacity, and rclone data
  path on the production Pi.
- Sustainable ZWO-to-hardware-H.264 throughput, copy count, and B&W bitrate/latency
  advantage at full source resolution.
