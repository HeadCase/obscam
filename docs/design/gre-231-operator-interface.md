# GRE-231 Production Operator Interface

## Summary

ObsCam is a camera-first, dark-adapted, single-screen viewer. Its primary desktop
form is a small monitoring window alongside other observatory telemetry. A healthy
resting viewer shows the complete uncropped feed and nothing else except a subtle
exposure rail at exposures of one second or longer. Interaction reveals status and
actions without resizing the feed.

This specification is the approved design input to GRE-223. It does not implement
the production UI or change server, HTTP, WebSocket, media, telemetry, or evidence
schemas.

## Approved amendments

The operator approved these changes to earlier issue wording:

1. Hidden controls no longer leave textual status permanently visible. After five
   seconds of inactivity, all chrome and the pointer hide. Exposure and degraded
   state edge cues remain because they provide at-a-glance operational truth.
2. `Capturing` is no longer a primary feed state. Acquisition activity is
   independent of feed currency; `Waiting for first image` names the initial
   no-frame condition.
3. The raw Quality JSON download is removed from the operator interface. The same
   authoritative evidence remains available to read-only agent diagnostics, and
   human-readable values remain available in the expandable status panel.
4. Optional PWA work is deferred. Any future PWA must retain the dark launch surface
   and must never imply that camera monitoring works offline.
5. GRE-184's persistent fourteen-button exposure grid and gain slider are replaced
   by a compact settings popover. Exposure uses a fourteen-position detented slider;
   the user-facing sensitivity label is `ISO`. The WebSocket and camera boundary
   continue to use the existing gain field unchanged.

These amend presentation requirements in GRE-218, GRE-219, GRE-223, and GRE-231.
They do not require a wire-schema change.

## Information hierarchy

### Resting viewer

The healthy sub-second resting state contains only the native-aspect-ratio feed.
The cursor is hidden on pointer devices. The source remains `object-fit: contain`;
black letterbox space is acceptable and must never be replaced with crop, rotation,
zoom, or image scaling policy.

At exposures of one second or longer, a two-pixel low-luminance blue-gray rail at
the bottom edge fills left to right from authoritative exposure start to expected
exposure completion. It holds full while the completed generation awaits exact
browser presentation. Exact presentation produces a 300 ms soft rail brightening,
then the rail resumes the truthful progress of the exposure already in flight.

When the feed is not healthy, the progress rail yields to an inset state boundary:

| Feed condition | Resting cue |
| --- | --- |
| Live, exposure below 1 s | No chrome |
| Live, exposure at least 1 s | Blue-gray bottom exposure rail |
| Waiting for first image | Bottom exposure rail on the black feed surface |
| Reconnecting | Low-luminance dashed amber perimeter |
| Stale or freshness unknown | Low-luminance dashed amber perimeter |
| Unavailable | Muted-red double perimeter |

The feed pixels are never dimmed, tinted, blurred, watermarked, or otherwise
altered. State boundaries use geometry as well as colour. Reduced motion removes
interpolation while retaining current rail position and static boundary geometry.

### Revealed viewer

Activity reveals one compact bottom command bar. It shows feed state, frame age only
when it helps the operator, visible exposure, ISO, treatment, Snapshot, and Take
control. The mini-viewer target keeps this bar to one line.

The state text has a semantic companion mark: filled blue for Live, hollow blue for
Waiting for first image, amber for Stale or Reconnecting, and red for Unavailable.
The text always carries the precise meaning; the mark supports rapid scanning and
never remains a constant decorative colour across different states.

The operator interface contains no technical diagnostics panel. It translates
authoritative evidence into actionable language such as `Live`, `Freshness unknown`,
`Reconnecting`, `Frame overdue`, or `Camera unavailable`. Correlation ratios,
cadence statistics, latency percentiles, uncertainty, epochs, generations, and raw
Quality JSON remain read-only agent diagnostics rather than user-facing metrics.

### Controller

Successful takeover keeps the same single-line command bar, substitutes Release for
Take control, and immediately opens a temporary settings popover. The command bar's
standalone Settings button remains the one control for reopening that popover.

The popover contains matching discrete sliders for Exposure and ISO, plus a direct
B&W/Colour segmented choice. Exposure has fourteen evenly spaced positional locks
ordered from 10 ms to 30 s. ISO has thirteen evenly spaced locks for 0, 50, 100, …,
600. Both feel continuous during drag and always land on a valid camera value.
Moving a control changes a browser-local draft only; it never submits on release.
Arrow keys move one detent.

The desktop panel uses two dense rows: label, rail, value. Treatment, `Discard`, and
`Apply` share a compact footer. Apply is disabled until the local draft differs from
the newest requested tuple, then sends the complete draft tuple exactly once.
Discard restores the newest requested tuple. The panel omits a title and authority
explanation; the separate `Release` command already communicates that control is
held. Coarse-pointer layouts preserve 44 CSS px interaction targets rather than
inheriting the denser desktop row height.

The Settings button toggles the popover; Escape, outside activation, loss of
authority, or the normal inactivity timeout may also hide it. Hiding settings never
releases control and preserves any local draft.
The compact values are an unboxed readout, not part of the settings control. A
separate button labelled `Settings` reopens the popover; there is no unexplained
disclosure glyph. The button uses the blue focus-colour border, raised control
surface, and hover/focus response while authority is held. In Viewer mode that same
button remains in place but is disabled with muted text, a neutral border, and no
hover response. This prevents authority changes from shifting the readout, Snapshot,
or adjacent commands. `Release` remains a separate action. Video geometry never
changes when either surface appears.

## Feed-state model

Feed state and exposure activity are independent.

| Primary state | Meaning | Image treatment | Resting cue |
| --- | --- | --- | --- |
| Live | Decoded browser media is advancing within the expected delivery contract; exact identity may still be pending | Retain exact pixels | None or exposure rail |
| Waiting for first image | The first authoritative exposure is within deadline and no decoded presentation exists | Black feed surface | Exposure rail |
| Reconnecting | Browser media transport is restoring | Retain the last trustworthy frame when available | Dashed amber perimeter |
| Stale | A newer presentation is overdue or freshness cannot be proven | Retain the last trustworthy frame indefinitely | Dashed amber perimeter |
| Unavailable | Capture or delivery is confirmed unusable and no working exposure can produce a trustworthy presentation | Retain a prior trustworthy frame if one exists; otherwise black | Muted-red double perimeter |

The status strip supplies the exact reason on interaction. A normal 30-second
exposure remains Live while the retained media keeps advancing; the exposure rail
explains why its source image is not changing. Exact identity remains a separate
diagnostic fact. The state
becomes Stale only after exposure duration plus the authoritative delivery allowance
passes without the expected presentation.

## Settings interaction

Exposure, internal gain, and treatment always travel as one complete tuple. The UI
labels the gain detents as ISO without claiming calibrated photographic ISO.

1. Opening settings initializes the local draft from the newest requested tuple.
   Moving Exposure, ISO, or Treatment mutates only that draft.
2. Draft values use Tokyo Night purple. A blue positional marker remains at the
   value used by the visible frame. If an older request is still awaiting visibility
   while a newer draft is edited, a smaller yellow marker retains that requested
   position.
3. Apply is enabled only while the draft differs. Activating it sends Exposure,
   internal gain, and Treatment as one complete tuple, once, through the existing
   command. It does not infer completion from slider release or an inactivity timer.
4. Discard restores the newest requested tuple without contacting the server.
5. After `Accepted`, controls remain available even while the generation is awaiting
   `Applied` or `Visible`; a newer applied tuple may supersede the older one.
6. While a requested setting differs from its visible value, a blue positional
   marker remains at the value used by the currently visible frame and the requested
   thumb occupies its new detent in Tokyo Night yellow. Only that setting's value
   text turns yellow in the panel and compact summary; ISO is treated as the semantic
   pair `ISO 300`, so its name and number change together. Other labels, unchanged
   values, and the enclosing border do not change. The interface does not narrate
   protocol phases such as Accepted, Applied, or Visible in normal operator text.
7. `Applying` uses an indeterminate indicator until authoritative exposure start.
   Exposure start then switches to determinate progress. Expected completion does
   not imply browser visibility; the presentation event alone advances `Visible`.
8. Exact frame presentation moves each changed setting's blue visible position to
   its requested detent; the old marker disappears and thumb and value return to
   blue. This is the only visual claim that the new tuple has reached the screen.
9. Exposure and ISO controls contain only accepted detents. Keyboard navigation
   changes the local draft one detent at a time; Enter may activate Apply.
10. Rejection briefly renders the requested control in red, then snaps it back to the
   blue visible position. Displacement, expiry, or reconnect similarly discards
   unsent intent and restores authoritative values. Mutations are never silently
   queued for later delivery.

## Authority interaction

- Every session begins as a Viewer.
- If no other viewer controls the camera, `Take control` grants authority directly.
- If another viewer controls the camera, the same direct action performs immediate
  generation-fenced takeover without a confirmation dialog.
- Successful takeover opens settings immediately.
- The Settings button, Escape, outside activation, and automatic chrome hiding retain
  authority and preserve a local draft. Settings reopens without another takeover.
- A displaced controller immediately collapses to viewer mode, discards unsent
  intent, restores authoritative settings, and announces `Control taken by another
  viewer` until the normal hide timeout.
- Release is immediate, needs no confirmation, collapses to viewer mode, and never
  interrupts the feed. It discards an unsent draft and briefly reports that fact.
- While disabled in Viewer mode, Settings exposes the hover text `Take control to
  change settings` and the equivalent accessible description.
- Visual inactivity never releases a healthy lease. Media and control connectivity
  remain independent.

## Snapshot interaction

Snapshot is available to Viewers and Controllers. It captures only the currently
visible native-dimension image and never includes controls, status, margins, state
boundaries, or the exposure rail. A trustworthy stale frame remains downloadable.

Where a secure supported browser exposes `showSaveFilePicker`, Snapshot opens Save
As with the truthful filename suggested. Elsewhere it uses a standard browser
download and lets browser configuration govern the destination. Cancellation is
not reported as failure.

Filenames include exact capture time and source generation when known. When exact
correlation is unavailable, the filename explicitly says `correlation-unknown`.
Successful save/download receives a brief non-blocking acknowledgement while the UI
is revealed; failures remain visible and actionable.

## Reveal and auto-hide contract

- Pointer movement or entry, touch, and keyboard input reveal chrome immediately.
- A bare-feed tap toggles chrome. A control tap performs only that control action.
- Chrome hides five seconds after the last genuine interaction.
- The countdown resets on pointer movement, touch, or keyboard input.
- Chrome cannot hide while a control is hovered, focused, pressed, or dragged.
- When protected interaction ends, the five-second countdown resumes.
- Reveal and hide use a 150 ms opacity fade only. The feed never moves or resizes.
- `prefers-reduced-motion: reduce` makes the transition immediate.
- Pointer inactivity also hides the cursor; movement restores cursor and chrome.
- State changes while inactive update the edge cue but never reveal full chrome.
- Automatic hiding preserves an unsent local settings draft while authority remains
  valid; the Settings button identifies that unapplied work when chrome is revealed.
- Background tabs suspend cosmetic animation. Foreground return reconnects and
  recomputes rail position and feed state from authoritative time.

## Responsive layouts

### iPhone 15 Pro Max portrait — 430 x 932 CSS px

- The 16:9 feed remains horizontally maximal and vertically centered when resting.
- The command bar uses the bottom safe area and available letterbox space first.
- The settings popover stacks labelled detented sliders with at least 44 x 44 CSS px
  targets and may cover the feed temporarily.

### iPhone 15 Pro Max landscape — 932 x 430 CSS px

- The feed remains fixed and contained across the viewport.
- The command bar and temporary settings popover overlay the feed.
- Safe-area insets protect all interactive controls.
- Controls remain reachable without internal horizontal scrolling.

### MacBook Pro mini viewer — target 756 x 490 CSS px

- This is the primary desktop layout and represents a quarter-screen monitoring
  window alongside other observatory telemetry.
- Resting presentation prioritizes the camera tile; revealed viewer and controller
  chrome remains one compact command bar.
- The settings popover is content-sized and anchored above the Settings button.

### MacBook Pro expanded viewer — target 1512 x 982 CSS px

- The feed remains centered at native aspect ratio.
- The command bar stays compact rather than stretching edge to edge.
- The settings popover remains content-sized and anchored to the command bar.

Viewport resize never changes the selected settings, authority, pending intent, or
feed state. It only recomputes containment and overlay placement.

## Visual language

The only production appearance is dark-adapted dark mode. Colours use the canonical
[Tokyo Night Night palette](https://github.com/folke/tokyonight.nvim/blob/main/extras/lua/tokyonight_night.lua).

| Token role | Starting value | Use |
| --- | --- | --- |
| Canvas | `#15161e` | Page and dark loading surface |
| Chrome | `#1a1b26` at 96–98% opacity | Status and control surfaces |
| Control surface | `#292e42` | Buttons, inputs, quiet hover |
| Primary text | `#c0caf5` | Essential labels and values |
| Secondary text | `#a9b1d6` | Detail and units |
| Border / progress | `#414868` | Quiet structure and healthy exposure rail |
| Current / focus | `#7aa2f7` | Visible setting, selection, focus, frame arrival |
| Draft | `#bb9af7` | Browser-local setting not yet sent |
| Requested | `#e0af68` | Setting requested for a future frame |
| Degraded | `#ff9e64` | Stale, reconnecting, unknown |
| Rejected | `#f7768e` | Rejected requested setting |
| Unavailable | `#db4b4b` | Confirmed unavailable state |

Values are canonical palette assignments and must be checked on the actual iPhone
and Mac displays at low brightness. The cool blue is limited to small, low-area
interactive and current-state marks; no pure-white surface, bright loading flash,
or ambient light-mode transition is allowed. The page background,
browser theme colour, overscroll, form controls, save feedback, error surfaces, and
any future launch screen must remain dark-adapted.

Use system UI fonts, tabular numerals for timings and settings, sentence case, and
text labels with supplementary icons. Essential meaning never depends on colour or
animation. Minimum touch target is 44 x 44 CSS px. Keyboard focus is never hidden by
auto-hide.

## Accessibility

- One `main` landmark contains a labelled camera region, status region, and controls.
- Feed state uses a polite live region; takeover, displacement, setting rejection,
  and save failure use concise announcements.
- Accepted, Applied, and Visible announcements are coalesced per generation so fast
  transitions do not create screen-reader chatter.
- Native buttons and range inputs retain platform semantics. Exposure and treatment
  selections use `aria-pressed`; visible and requested settings have distinct names.
- Focus order is feed state, settings readout, Settings, Snapshot, and Take control
  or Release.
  Popover controls follow the settings trigger and DOM order matches visual order.
- Escape closes the settings popover and returns focus to its trigger.
- Focus, hover, selection, pending, disabled, degraded, and unavailable states use
  shape or text in addition to colour.
- Zoom to 200%, increased text size, VoiceOver, keyboard-only operation, and reduced
  motion must preserve complete access without horizontal page scrolling.

## GRE-223 implementation handoff

GRE-223 should implement this design around the existing reducer/effect boundary.
GRE-232 owns the first focused slice: responsive browser-local drafting, explicit
full-tuple Apply, truthful Draft/Requested/Applied/Visible feedback, and the binding
GRE-189 settings-transition measurements. GRE-223 owns the surrounding production
viewer and integrates that slice.
Required browser-side changes are:

1. Split feed-state projection from exposure activity and replace primary Capturing
   with Waiting for first image.
2. Add the five-second chrome/pointer visibility state as browser-local UI state.
3. Render the single Viewer/Controller command bar and temporary settings popover
   without changing video geometry.
4. Render the exposure rail and exact-presentation acknowledgement from existing
   authoritative capture and correlation facts.
5. Add degraded edge cues without modifying video pixels or snapshot output.
6. Add the detented Exposure and ISO sliders with spatial draft/requested/visible
   state, explicit full-tuple Apply, and responsive superseding input.
7. Implement progressive Save As with standard-download fallback.
8. Remove technical service-quality metrics and Quality JSON from the operator UI
   while preserving the read-only evidence endpoint for agent diagnostics.
9. Do not add PWA assets in this work.

No server-side UI state, media path, snapshot endpoint, persistence, accounts,
rotation, crop, or scaling behavior may be introduced.

## Verification matrix

The implementation and later browser qualification must exercise:

- all four representative viewport sizes;
- Viewer, Controller, another-controller, displaced, expired, and reconnecting
  authority;
- first exposure, sub-second Live, 1 s and 30 s exposure progress, exact frame
  arrival, interrupted exposure, and overdue delivery;
- exact and unknown correlation, trustworthy stale retention, Reconnecting, and
  Unavailable with and without a retained frame;
- Accepted, Applied, Visible, rejection, and rapid superseding settings;
- five-second timeout, pointer restoration, bare-feed touch toggle, hover/focus/drag
  protection, resize, background, and foreground return;
- snapshot Save As, fallback download, cancellation, unknown-correlation naming, and
  failure;
- keyboard, VoiceOver, reduced motion, safe areas, 200% zoom, contrast, and dark
  loading/error surfaces;
- confirmation that edge cues and overlays never appear in snapshot pixels.

Production qualification remains GRE-229 and must use current Zen, Safari, and a
Chromium-family browser plus the required four-viewer LAN/WireGuard matrix.

## Assumptions and deferrals

- The bottom exposure rail is approved without competing prototype variants.
- Exact palette tuning is validated on the operator's physical displays.
- The raw evidence response and technical metrics remain agent-readable without an
  end-user diagnostics panel or JSON button.
- PWA installation, offline shell behavior, and install guidance are deferred.
- Decorative themes, user customization, recording, history, zoom, rotation, and
  server-side snapshots remain out of scope.
