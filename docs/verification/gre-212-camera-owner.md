# GRE-212 camera-owner verification

## Automated substitute

The `zwo-asi` crate's `sdk-stub` feature links a C implementation of the exact
SDK symbols used in production. Tests exercise the public camera-owner seam and
cover exact-model selection, ASI178MC non-ownership, serial/dimension/Bayer/format
rejection, lifecycle failure normalization, the full settings envelope,
four-buffer reuse, generations, SDK drops and drop-query failures, capture
errors, bounded interruption latency, stop, and single-attempt close ownership.

The production build does not enable `sdk-stub` and links `/usr/local/lib/libASICamera2.so`.

## Deployed hardware

Status: **passed on 2026-07-30** against the production SDK and fixed two-camera
Raspberry Pi installation.

- Exact `ZWO ASI662MC` model and factory serial `1d274e0920010900` validated.
- Five consecutive 1920×1080 RAW8 RGGB frames at 10 ms/gain 0 produced
  generations 1–5, exact 2,073,600-byte lengths, zero SDK drops, and pointer
  reuse from frame 1 to frame 5, proving rotation through the fixed four buffers.
- Gain 600 produced a further complete generation at 10 ms.
- Caller interruption during a 30 s exposure returned `Interrupted` after
  100 ms, matching the owner's bounded SDK wait.
- A real 30 s/gain 600 integration completed after 30,161 ms as generation 7,
  with the exact frame length and zero SDK drops. Bounded SDK timeouts were
  tolerated while the long integration remained in progress.
- Stop and close completed cleanly, followed by three additional exact-identity
  open, initialize, configure, start, capture, stop, and close cycles.
- Before and after verification, `allsky.service` remained active with main PID
  5905, zero restarts, and the same ASI178MC `capture_ZWO` PID 6025.
- ASI178MC remained at USB path `1-1.1.4` at 480 Mbit/s; ASI662MC remained at
  `2-2` at 5000 Mbit/s. No disconnect or re-enumeration was observed.
- The kernel recorded the known SDK-enumeration claim on the ASI178MC interface
  and ASI662MC SuperSpeed resets. AllSky ownership and both fixed USB identities
  remained continuous, so this did not constitute displacement.

Physical unplug/replug remains outside the accepted fixed-installation scope.
Logical owner shutdown and revalidation/reopen were exercised instead. Vendor
failure normalization remains covered by the linkable stub because deliberately
injecting real camera or USB faults would be destructive to observatory service.
