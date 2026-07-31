# Graphical Browser Testing

ObsCam runs on a headless Raspberry Pi, while its graphical test browser runs
on the operator's Mac. Local browser executables on the Pi are neither required
nor expected.

## Topology

```text
Codex and ObsCam service on the Pi
        |
        | SSH tunnel carrying the Playwright MCP connection
        v
Playwright MCP and browser on the Mac
        |
        | WireGuard through wg0
        v
ObsCam on 10.164.190.1:<service-port>
```

- Agents control the Mac browser with the `playwright_mac` MCP tools.
- The SSH tunnel carries the MCP connection between Codex and the Mac. It is
  not the route used by the browser to reach ObsCam.
- The browser reaches ObsCam on the Pi through WireGuard `wg0` at
  `10.164.190.1`.
- When the LAN route is appropriate, the only fallback service address is
  `192.168.1.200`.
- Navigating to the LAN address does not by itself prove physical-LAN ingress;
  record the observed client route. A browser whose source remains in the
  WireGuard range verifies the LAN destination mapping, not an Andrew-style
  device physically attached to the observatory LAN.
- `localhost` or `127.0.0.1` in a Playwright navigation refers to the Mac, not
  the Pi, and must not be used for the Pi-hosted service.

## Protected Interface

Do not use, inspect, probe, test, reconfigure, or otherwise touch `wg1`
(`192.168.4.9`). It is an unrelated tunnel back to the operator's home LAN for
writing astrophotography images to the NAS. It is never an ObsCam browser-test
route or fallback.

## Verification Procedure

1. Start the ObsCam service on the Pi, listening on an address reachable through
   `wg0` and the LAN, normally `0.0.0.0:<service-port>`.
2. Confirm that the `playwright_mac` MCP tools are available. Do not check for a
   local Pi browser as a proxy for this capability.
3. Navigate the Mac browser to
   `http://10.164.190.1:<service-port>`. Use
   `http://192.168.1.200:<service-port>` only as the LAN fallback.
4. Exercise the browser-facing contracts and inspect the rendered accessibility
   tree, mobile and desktop viewports, console messages, and network requests.
5. Report browser verification as unavailable only after the MCP connection and
   both permitted service routes have been checked. Never involve `wg1` in that
   diagnosis.

## Playwright MCP Constraints

Code passed to `browser_run_code_unsafe` executes in a restricted VM rather
than a normal Node.js module. Do not assume Node globals such as `URL` or
`setTimeout` exist there. Prefer Playwright-owned operations such as
`page.waitForTimeout()` and compare `response.url()` as a string. In particular,
an exception thrown from an asynchronous page event listener can escape the
tool call and terminate the Playwright MCP process; keep listener callbacks
minimal, avoid unavailable globals, and remove temporary listeners when the
check finishes.

The MCP Chrome session does not reliably report a page as hidden when another
page is brought to the front. To exercise ObsCam's foreground-reconnect browser
contract deterministically, install a test-controlled `document.visibilityState`
getter on that isolated test page and dispatch `visibilitychange` for hidden
and visible transitions. Assert the observable contract: a new media
connection, Reconnecting before Live, and advancing video after reconnection.
Do not claim that this technique verifies native window-manager tab semantics.

MCP console and network history can include earlier navigations and failed
diagnostic runs. Acceptance checks should attach fresh page listeners before
navigation and judge only events collected by those listeners. Use all-session
MCP logs for diagnosis, not as isolated pass/fail evidence.

## Repository Playwright Suite

The repeatable graphical acceptance suite lives in `web/e2e`. Run it from an
operator-Mac checkout while the intended release build and MediaMTX are running
on the Pi:

```sh
npm ci
npx playwright install chromium
OBSCAM_BASE_URL=http://10.164.190.1:8080 npm run test:browser
```

Use `http://192.168.1.200:8080` only for the permitted LAN fallback described
above. The suite is serial because camera settings and control authority are
shared appliance state. Every settings test restores the tuple it observed and
releases authority. Playwright retains traces, screenshots, and video for a
failing test in `test-results`, with the HTML report in `playwright-report`.

The browser suite is a required quality gate for browser behavior changes. A
Pi-side typecheck or reducer test does not substitute for running Chromium on
the Mac against the deployed Rust, FFmpeg, and MediaMTX path.

## GRE-211 Known-Good Check

The truthful unavailable viewer was verified from the Mac browser against
`http://10.164.190.1:8080` with a `390x844` mobile viewport and a `1440x900`
desktop viewport. The browser observed:

- HTTP 200 responses from `/api/v1/runtime` and `/api/v1/health`
- schema version 1 and one shared runtime epoch
- independently unavailable capture, encoder, and relay components
- `latestFrame: null`
- a rendered `Unavailable` state with no frame age, cadence, generation, or
  latency
- all fourteen exposure choices and all unavailable controls disabled
- no horizontal or vertical viewport overflow

The browser also requested `/favicon.ico`, which currently returns HTTP 404.
That does not contradict GRE-211's acceptance criteria, but it remains visible
as a console error and should be addressed by later browser-shell work.
