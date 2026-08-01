# GRE-222 independent browser-transport recovery verification

## Implemented contract

Each browser owns separate reconnect loops for its WHEP/WebRTC media session and
control WebSocket. Each loop has its own connection generation, failure count,
retry timer, stability timer, and cancellation. The first retry is immediate;
later retries use equal-jitter exponential backoff capped at five seconds. A
connection that remains open for five seconds resets only its own backoff.
WHEP negotiation is bounded to ten seconds and replacement or page teardown
cancels ICE gathering, HTTP negotiation, remote description, and track delivery.

Foreground and browser `online` events cancel accumulated delays and replace
both connections immediately. Authoritative media-recovery facts replace only
the WHEP connection. Closing a page cancels both loops and fences every delayed
callback.

Control connection generations are distinct from lease generations. Control
loss immediately clears validated authority and pending browser intent while
retaining only same-runtime resumable credentials. Reconnection may send a
credential-bearing `resume`; settings mutation remains reachable only from a
direct enabled control action. The UI reports whether control is reconnecting
immediately or after a bounded retry delay.

Each WHEP replacement advances the browser media-connection generation before
negotiation, and negotiation cannot mutate the visible video. Old peer
callbacks, presentation callbacks, and frame mappings cannot affect the new
connection. Before replacing the accepted stream, the browser copies the last
trustworthy pixels into a browser-local canvas overlay once per source
generation, never once per repeated H.264 presentation. The overlay remains
visible until a newly correlated current-generation presentation arrives.

## Automated evidence

- Reconnect-loop tests use deterministic clock and randomness boundaries to
  prove immediate retry, bounded jitter, exponential growth, stable reset,
  forced-delay bypass, generation fencing, and final cancellation.
- Control reducer tests prove disconnect immediately disables mutation,
  retry detail is represented explicitly, resumable credentials retain no
  pending intent, and stale connection callbacks cannot restore authority.
- Viewer reducer tests prove new media generations invalidate old evidence and
  stale connected/disconnected callbacks cannot change the current session.
- The HTTP contract test proves the qualified Rust binary embeds and serves the
  reconnect module alongside the rest of the browser asset set.
- The repository Playwright reconnect suite covers initial and repeated WHEP
  failure, bounded hung negotiation, control-only loss, mutation non-replay,
  stale same-runtime credentials, foreground return, and browser network
  restoration.

## Browser and deployed-stack evidence -- 2026-08-01

The release Rust service used the real ASI662MC, hardware H.264, the pinned
MediaMTX v1.19.3 checksum/configuration/network boundary, and Mac Playwright
Chromium over the permitted `http://10.164.190.1:8080` WireGuard route. Health
reported capture, encoder, and relay Ready, and the private metrics endpoint
reported `paths{name="obscam",state="ready"} 1`.

- A browser route rejected four consecutive WHEP POSTs. Control authority was
  acquired while media was still recovering, remained `You have control`, and
  the fifth WHEP attempt returned the viewer to Live.
- A separate route held the first WHEP POST beyond the ten-second negotiation
  deadline. The browser cancelled it, made a second attempt, and returned to
  Live without refresh.
- A routed control connection was closed and the following attempt was also
  rejected. During the bounded retry the control UI reported reconnecting,
  mutation controls were disabled, and otherwise healthy video remained Live.
  The third connection resumed the lease.
- One settings mutation was submitted before that control failure. Exactly one
  mutation was observed before and after reconnect; resume did not replay it.
  The original exposure was restored and authority released after the check.
- A hidden-to-visible transition advanced WHEP and control connection counts
  independently from one to two, showed Reconnecting before Live, kept the
  unavailable-frame overlay hidden, displayed a native 1920x1080 retained-
  frame canvas whose sampled pixels exactly matched the last trustworthy
  presentation, hid that canvas only after exact current-generation
  correlation, and resumed advancing video.
- Four deliberately rejected WHEP negotiations established a quiet scheduled
  media backoff; `online` cancelled it and produced a request in 207 ms rather
  than waiting for the next delay of at least 500 ms. A separate four-failure
  control check established its own quiet scheduled backoff and `online`
  produced the next WebSocket in 11 ms. Both returned to Live.
- A syntactically valid same-runtime credential with a stale lease generation
  sent only `resume`, never sent `set_settings`, and was removed from tab
  storage after server rejection.

A Mac Chromium microbenchmark copied the native 1920x1080 video frame into a
canvas 200 times and forced a pixel readback. It took 20.8 ms total, or 0.104 ms
per copy. At 100 distinct source generations per second that is approximately
1% of one browser main-thread second; repeated H.264 presentations do not copy.

The first deployed navigation found that the new compiled module was not yet
served by Rust. A red HTTP contract test reproduced the 404; the asset was then
added to the qualified embedded set, and the rebuilt release passed the same
Mac browser path without that error.
