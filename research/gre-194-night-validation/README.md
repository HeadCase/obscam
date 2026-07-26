# GRE-194 nighttime long-exposure integration evidence

## Decision

Use SDK video acquisition across the production exposure range, including
30-second exposures. Under genuinely dark conditions, video and snapshot
acquisition produced the same exposure-response curve and statistically
indistinguishable full-frame RAW8 signal at 30 seconds. Snapshot acquisition is
not required to obtain a true long exposure.

This experiment answers whether a frame returned through the SDK video API at a
30-second setting actually integrates the scene like the SDK snapshot API. It
does not repeat GRE-183's endurance qualification or establish interruption
semantics.

## Method

- Deployed Raspberry Pi 4 and production ASI662MC at USB 3 SuperSpeed
- ZWO SDK `1, 38, 0, 0`
- Full-frame 1920x1080 RAW8, gain 400, bandwidth 50, high-speed mode off
- Exposures of 1, 5, 10, and 30 seconds
- Three video frames and three snapshot frames at each exposure
- Acquisition order alternated within each matched set to reduce sky-brightness
  drift
- A 100 ms sysfs sentinel aborted a runner if the production ASI178MC
  disappeared; AllSky was not stopped and the ASI178MC was never opened

Gain 400 was selected from a preceding nighttime scout across gains 0, 100,
250, 400, and 600. Its 30-second video frame had mean 26.680 DN with 0.155%
saturated pixels and no zero-valued pixels. Gain 600 was rejected because it
raised saturation to 7.820% and introduced 0.137% zero-valued pixels.

## Results

| Exposure | Video mean DN (range) | Snapshot mean DN (range) | Video vs snapshot | Video / snapshot call |
| --- | ---: | ---: | ---: | ---: |
| 1 s | 21.161 (21.150-21.177) | 21.157 (21.143-21.169) | +0.017% | 1,250.8 / 1,256.4 ms |
| 5 s | 22.008 (22.003-22.013) | 22.014 (22.011-22.019) | -0.031% | 5,269.4 / 5,274.9 ms |
| 10 s | 22.723 (22.698-22.740) | 22.728 (22.719-22.738) | -0.021% | 10,316.3 / 10,272.6 ms |
| 30 s | 26.252 (26.062-26.356) | 26.296 (26.234-26.354) | -0.168% | 30,270.9 / 30,270.8 ms |

Both acquisition modes accumulated progressively more signal as exposure rose.
At 30 seconds their mean signal differed by 0.044 DN, or 0.168%, while the
video repeats themselves spanned 0.294 DN. The mode difference is therefore
smaller than ordinary repeat-to-repeat scene variation. Their mean saturated
fractions were likewise effectively identical: 0.15125% for video and 0.15130%
for snapshot.

All 24 matched cells reported zero capture errors, SDK drops, corrupt frames,
and adjacent duplicates. Peak temperature was 60.374 C, peak runner RSS was
20,307,968 bytes, and the Pi reported `throttled=0x0` after the experiment.

## Interpretation

The SDK video path does not return a short, repeatedly sampled frame merely
delayed by the 30-second setting. Its signal increases with requested exposure
and matches the signal returned by the snapshot path at every tested point,
including 30 seconds. The approximately 30.27-second SDK call duration also
matches the integration period plus the already-characterized transfer cost.

The production capture service therefore needs one video acquisition path, not
separate video and snapshot paths selected by exposure length. Snapshot mode
has no measured image-integration role.

## Operational observations and limits

AllSky remained active, both cameras remained present after the experiment, and
the ASI178MC sentinel never fired. Kernel logs nevertheless recorded repeated
ASI662MC resets at runner open/close boundaries and recurring ASI178MC resets
while AllSky continued producing images. These shared-host USB events remain
reliability evidence for GRE-191; they do not change the matched integration
result.

The experiment compares full-frame aggregate signal and clipping statistics. It
does not measure calibrated per-pixel linearity, dark-current subtraction,
long-run error tails, camera reconnect behavior, stale-frame transitions, or
multi-client delivery.

Raw artifacts are under [`results/`](results/).
