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
