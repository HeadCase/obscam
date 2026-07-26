# GRE-196 low-noise monochrome evidence

This directory records evidence from the GRE-190 native browser harness extended
for GRE-196. The extension feeds each selected RAW8 generation to independent,
single-frame JPEG pipelines:

- `colour`: Bayer-derived colour converted to YUV420
- `mono`: Bayer-derived monochrome
- rejected during evaluation: monochrome with a light spatial Gaussian blur

The retained treatments use neutral rendering so comparisons are not confounded by
exposure-specific presentation adjustments.

No treatment retains a previous frame. All emitted envelopes include the source
generation and treatment so equivalent generations can be compared without
introducing temporal ghosting.

Start the native harness on the Raspberry Pi:

```bash
cargo build --manifest-path \
  src/obscam/tools/gre_190_prototype/rust_backend/Cargo.toml
python -m obscam.tools.gre_190_prototype serve --native \
  --fps 20 --exposure-us 10000 --gain 0
```

Open one foreground-visible Safari or Chromium tab per treatment through the
same LAN or WireGuard route, changing the `client` value per tab:

```text
http://PI:8190/?auto=1&run=RUN&client=colour&treatment=colour&duration=60
http://PI:8190/?auto=1&run=RUN&client=mono&treatment=mono&duration=60
```

Repeat at representative short and long closed-roof exposure/gain settings.
Record server process CPU/RSS alongside `GET /api/runs/RUN`. Use matched
generation numbers for visual and image-statistic comparisons. Include a moving
telescope or equivalent edge target and the built-in reconnect event in every
candidate-setting run.

## Decision

Retain neutral colour and neutral monochrome as the two v1 presentation
options. A light, single-frame Gaussian blur was compared against plain
monochrome from equivalent RAW8 generations under both red observatory lighting
and dark closed-roof conditions. It produced no discernible improvement in
telescope visibility or noise presentation, so its processing and product
complexity are not justified.

The comparison also rejected a hard-coded gamma/contrast/brightness stretch:
it made the red-light scene unnecessarily high contrast and coupled presentation
to one exposure condition. Exposure-aware treatment variation remains a
separate, non-blocking experiment in GRE-199. Persistent defective-pixel
correction remains separate in GRE-197.
