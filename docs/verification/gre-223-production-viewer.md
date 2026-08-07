# GRE-223 production viewer verification

Verified on 2026-08-06 against the release-mode Rust service, real ASI662MC,
FFmpeg hardware encoder, MediaMTX, and the operator-Mac Chromium browser over
the approved `10.44.0.1` route.

## Visual differential

The checked-in GRE-231 prototype and production viewer were compared at
1512 x 982 CSS pixels. The corrected production controls measured:

- 12 exposure and 13 ISO detents;
- custom range appearance for both sliders;
- 0 px centre difference between the settings panel and command bar;
- a 6.4 px attachment gap;
- 0 px command-bar width change across setting values;
- no horizontal overflow.

Screenshots were visually assessed at 1512 x 982 desktop, 430 x 932 phone
portrait, and 932 x 430 phone landscape. The full 1920 x 1080 source retained
`object-fit: contain`; opening settings did not resize or crop it.

The deployed follow-up assessment removed the transient command-bar subtext,
reserved a stable 152 px desktop status slot, and measured 0 px settings-readout
movement between `Live` and `Waiting for first image`. At 430 x 932 the command
bar uses two explicit rows. Status, requested settings, Settings, Snapshot, and
Take control had no pairwise overlap at 430 x 932, 932 x 430, 756 x 490, or
1512 x 982, and every viewport retained exact document-width containment.

The removal of status subtext and synthetic `Applying` progress are approved
operator corrections to the earlier GRE-231 prose. Setting phase descriptions
remain associated with their controls for assistive technology, but are visually
hidden and are not independent live regions. This prevents protocol narration
from adding rows to the panel or generating three competing announcements.

## Exposure rail

The deployed 5 s to 1 s reproduction originally caught deterministic rewinds,
including 76.8% to the artificial 24% Applying width and a later 23% to 3%
sensor-transition reset. The artificial Applying animation was removed. The
camera owner now consumes the interruption associated with an applied target,
and the runtime withholds capture progress while the three hardware-observed
transitional frames remain ineligible for exact settings metadata.

The final release-mode trace passed repeated 5 s to 1 s transitions with no
mid-capture regression or Applying class. The rail remained hidden across the
untrusted boundary, then advanced from the first stable authoritative capture.
Exact completed-capture boundaries remain the only permitted reset.

Reduced-motion browser coverage holds the current position and updates the
determinate rail discretely rather than continuously interpolating it.

## Interaction checks

- Native pointer dragging moved the exposure slider from detent 3 to detent 9
  without the renderer fighting the thumb.
- Pressing Enter on a treatment button with an existing draft produced no
  settings submission.
- Apply emitted one complete tuple; Discard and Release restored the observed
  500 ms / ISO 100 / monochrome tuple and released authority.
- The Settings trigger precedes the popover controls in keyboard DOM order and
  retains its authority-held blue border.
- Save As and download snapshot paths produced native 1920 x 1080 PNG output;
  cancellation remained silent.

## Automated gates

- `npm test`
- `npm run typecheck`
- `npm run typecheck:browser`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-targets --all-features`
- `cargo deny check`
- `cargo machete`

The Mac browser was driven through Playwright MCP against the deployed stack.
The checked-in browser suite was typechecked, and the changed visual, pointer,
keyboard, reduced-motion, rail, snapshot, authority, and viewport paths were
exercised directly because the Mac MCP host does not expose a repository
checkout from which to invoke the suite runner.

## Integrated performance acceptance

Verified on 2026-08-07 with the integrated GRE-223 viewer at the approved fixed
1.5 Mbit/s encoder target and the 50 ms exposure floor:

- one viewer delivered approximately 1.52 Mbit/s of ObsCam media;
- two viewers delivered approximately 3.07 Mbit/s of ObsCam media in aggregate;
- four colour viewers delivered approximately 6.18 Mbit/s of ObsCam media and
  approximately 7.58 Mbit/s total on `wg0` (about 0.95 MiB/s, including
  non-media traffic);
- all four viewers reported `Live`, advanced decoded video time, and retained an
  identical 736 px command-bar width;
- monochrome and colour were both exercised at 50 ms / ISO 600;
- ten rapid connect-and-navigate cycles created ten distinct WHEP sessions and
  returned MediaMTX to zero readers after each normal browser departure.

The media session used MediaMTX's approved `192.168.1.200:8189` candidate while
the browser page used `10.44.0.1`; packet capture confirmed the remote browser as
`10.44.0.2`, so this traffic still traversed the approved WireGuard route. Only
`wg0` and the MediaMTX port were scoped. The protected `wg1` interface was not
inspected.

These measurements reproduce linear per-viewer bandwidth rather than the earlier
approximately 4 MiB/s saturation with one or two clients. The principal multiplier
was departed or replaced WHEP sessions remaining alive, not exposure-dependent
bitrate selection. Normal navigation, reload, visibility replacement, and live
reconnect now send a same-origin keepalive cleanup request; MediaMTX timeout remains
the fallback for abrupt browser-process death, where page lifecycle handlers cannot
run.
