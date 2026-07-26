# GRE-184 mobile viewer controls prototype

> PROTOTYPE — throwaway evaluation code, not production UI.

Three variants of the mobile viewer controls, switchable via `?variant=`, on
the existing `/` route.

## Run

```shell
uv run uvicorn obscam.api.main:app --reload
```

Open <http://127.0.0.1:8000/?variant=A> and use the floating switcher or the
left/right arrow keys. The variants are:

- `A` — bottom tray
- `B` — edge controls with radial actions
- `C` — command palette

Camera settings, colour mode, rotation, and control authority are simulated in
browser memory. Snapshot uses the current stream URL as a download source.

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
