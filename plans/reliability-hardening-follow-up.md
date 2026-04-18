# Reliability Hardening Follow-Up Plan

## Summary

This plan converts the review findings into a sequence of future implementation
sessions. It prioritizes reliability first, then truthfulness of API state,
then deployability of the browser UI. The plan includes public API and internal
interface changes, so it should be treated as an approval-required package
before execution.

## Implementation Tasks

1. Add explicit backend health and recovery state to the service layer.
   Introduce a small internal state model covering `starting`, `running`,
   `recovering`, `degraded`, and `stopped`, plus `last_error`,
   `consecutive_capture_failures`, and `last_recovery_attempt_at`. Move repeated
   runtime-failure handling out of the capture loop and into the backend so the
   backend can own teardown, reconnect, and status reporting.

2. Implement automatic recovery after runtime camera failures. When the capture
   loop crosses a fixed failure threshold of 5 consecutive capture errors,
   notify the backend instead of only backing off. The backend should stop
   capture, disconnect the camera, and attempt reconnects with fixed backoff
   intervals of 1s, 2s, and 5s. If all three attempts fail, keep the backend in
   `degraded` state and continue background retries every 30s until success. A
   successful recovery must reload cached settings, restart capture, clear the
   failure counter, and publish updated status.

3. Make `/api/connect` an immediate recovery trigger instead of a start-only
   no-op. If the backend is stopped, it should start as today. If it is
   `running`, it should return a no-op success payload. If it is `recovering` or
   `degraded`, it should force an immediate reconnect attempt and return whether
   that reconnect succeeded. `GET /api/status` should expose the new backend
   health fields so operators and the UI can distinguish healthy, recovering, and
   degraded states.

4. Fix shutdown behavior for long exposures by making capture interruption
   explicit. Extend the internal `CameraInterface` with `interrupt_capture() ->
None`. Call it from `ContinuousCaptureLoop.stop()` before `join()`. Change stop
   logic so the backend does not disconnect the camera until the worker has exited
   or a hard timeout has been reached. Use an exposure-aware join timeout of
   `max(2s, current_exposure + 1s)` capped at 35s. The synthetic camera should
   replace raw `sleep()` with an interruptible wait so tests can prove stop
   behavior. The ZWO backend should map `interrupt_capture()` to `stop_exposure()`
   and `stop_video_capture()`.

5. Replace optimistic settings success with accepted-then-applied semantics.
   `POST /api/settings` should validate with a typed Pydantic request model,
   reject malformed JSON types with FastAPI validation errors, reject empty or
   out-of-range payloads with `400`, queue only validated keys, and return HTTP
   `202` with `status: "accepted"`, `queued_settings`, `queued_version`,
   `applied_settings`, and `timestamp`. Do not persist queued settings
   immediately.

6. Add applied-settings tracking in the runtime and capture loop. When the
   capture thread successfully applies settings, it should publish an
   applied-settings event, persist the camera’s actual current settings, and
   advance an `applied_settings_version`. If application fails, it should leave
   persisted settings unchanged, record the error in backend status, and publish a
   failure state so telemetry does not imply success. Telemetry payloads should
   include `applied_settings`, `applied_settings_version`, `pending_settings`, and
   `settings_state` with values `idle`, `pending`, `applied`, or `failed`.

7. Harden the settings boundary and remove the current 500-on-bad-input path.
   Replace the ad hoc request-body parsing in `/api/settings` with a schema
   model that forbids unknown keys. Keep range validation capability-driven. Add
   explicit error messages for unsupported keys, empty payloads, and out-of-range
   values. Do not coerce arbitrary strings into numeric settings.

8. Make saved snapshots browser-fetchable. Mount the repo `assets/` directory
   read-only at `/assets` and keep the snapshot API returning the existing
   relative path for compatibility. Add a `url` field to the snapshot response
   equal to `/<relative_path>`. Preserve current subdirectory validation and
   filename sanitization.

9. Clean up the browser dependency chain for offline-safe operation. Remove
   unused HTMX from the template. Eliminate CDN dependence by serving Alpine
   locally from `frontend/static/` and replacing the Tailwind CDN dependency with
   repo-local styles in `frontend/static/css/app.css`. Do not introduce a
   Node/Tailwind build step; keep the frontend static and repo-contained.

10. Update operator-facing status and logs to match the new lifecycle. Log
    transitions into `recovering`, `degraded`, recovery success, recovery
    exhaustion, settings apply success, and settings apply failure. Expose recovery
    state in bootstrap data so the UI can show “recovering” instead of generic
    “telemetry error” messaging.

## Public API and Interface Changes

- `GET /api/status` gains backend lifecycle and recovery fields:
  `backend_state`, `last_error`, `consecutive_capture_failures`, and
  `last_recovery_attempt_at`.
- `GET /api/connect` becomes an idempotent start-or-recover endpoint with
  explicit reconnect behavior when unhealthy.
- `POST /api/settings` changes from immediate success semantics to HTTP `202
Accepted` with queued-versus-applied fields.
- Telemetry payloads gain `applied_settings`, `applied_settings_version`,
  `pending_settings`, and `settings_state`.
- `POST /api/snapshots` keeps `relative_path` and adds a browser-fetchable
  `url`.
- Internal camera interface adds `interrupt_capture()`.

## Verification

- Add pytest coverage for repeated capture failures triggering recovery,
  recovery success after temporary failure, and persistent failure ending in
  `degraded`.
- Add pytest coverage for stopping during a simulated long exposure and assert
  that the worker exits before camera teardown.
- Add pytest coverage for `/api/connect` behavior in `stopped`, `running`,
  `recovering`, and `degraded` states.
- Add pytest coverage for malformed settings payloads, unknown keys, empty
  payloads, out-of-range values, accepted queueing, successful apply, and failed
  apply without persistence.
- Add pytest coverage confirming snapshot responses include a fetchable `url`
  and that `/assets/...` is served by the app.
- Add a frontend smoke test or template-level assertion that no third-party CDN
  URLs remain in the rendered page.
- Run the full repo quality gate after each implementation session: `uv run
ruff check`, `uv run ruff format --check`, `uv run ty check`, `uv run deptry
.`, and `uv run pytest`.

## Assumptions

- Automatic recovery is the chosen behavior, even though it adds more lifecycle
  complexity than a manual-only reconnect model.
- Accepted-then-applied settings semantics are the chosen API contract, and the
  UI should eventually reflect queued versus applied state.
- Serving snapshots from `/assets/...` is acceptable even though it also
  exposes fixture images already stored under `assets/`.
- Hardware-dependent recovery behavior will be tested primarily through
  fake-camera seams in pytest, with real ZWO validation limited to manual sanity
  checks in a later hardware session.
