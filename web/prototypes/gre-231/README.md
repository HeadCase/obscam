# GRE-231 throwaway prototype

This prototype answers: **Does the approved feed-first operator interface remain
legible and operable across the mini viewer, mobile layouts, lifecycle states,
authority transitions, and long-exposure feedback?**

It is a design-validation artifact, not production code. GRE-232 owns the first
responsive-settings implementation slice; GRE-223 owns the complete production
viewer around the existing TypeScript reducer and browser effects.

Run it from the repository root:

```sh
npm run prototype:gre-231
```

Then open `http://127.0.0.1:4173`. Resize the browser for the representative
viewports in `docs/design/gre-231-operator-interface.md`. Take control to try the
matching detented Exposure and ISO sliders; the standalone Settings button hides and
reopens the panel without releasing control. Slider and Treatment edits remain
purple, browser-local drafts until the complete tuple is sent with Apply. The
scenario bar changes feed state, forces exact frame presentation,
simulates a rejected setting, and toggles the five-second chrome behavior.
