# GRE-184 mobile viewer controls

> PROTOTYPE — throwaway evaluation code, not production UI.

## Host guard

This prototype must run on the GRE-190 greenfield JPEG/WebSocket viewer at
`src/obscam/tools/gre_190_prototype`. The legacy `frontend/` application is not
an implementation host and must remain untouched.

The GRE-190 benchmark page remains available at `/`. GRE-184 is an adjacent
evaluation surface because the benchmark page is instrumentation rather than a
product viewer. Both routes use the same greenfield server and frame fan-out.

## Run

```shell
uv run python -m obscam.tools.gre_190_prototype serve --native --fps 20 --exposure-us 10000 --gain 0
```

Open <http://127.0.0.1:8190/gre-184?variant=A>. Use the floating switcher or
left/right arrow keys to compare:

- `A` — compact two-row bottom tray with direct exposure choices (preferred)
- `B` — edge controls with radial actions
- `C` — command palette

This command assumes the GRE-190 native backend prerequisites in `README.md`
are already built. The deterministic generated source feeds only the colour
fan-out and therefore cannot evaluate treatment switching.

B&W/colour switching reconnects the real GRE-190 treatment stream, and snapshot
download exports the actual presented canvas. Exposure, gain, and authority are
browser-local simulations because the greenfield delivery prototype deliberately
has no camera-control API.

## Evaluation tasks

For each variant on a mobile viewport, record completion time, errors or
reversals, and confidence while asking the participant to:

1. Set a specified exposure and gain.
2. Switch between B&W and colour.
3. Download a snapshot.
4. Take control from the simulated other viewer.
5. Rotate the display.
6. Wait for auto-hide and recover the controls.
7. Confirm that every part of the source frame remains visible.
