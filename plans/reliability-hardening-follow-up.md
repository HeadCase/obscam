# Reliability Hardening Follow-Up Plan

## Summary

This revised plan converts the review findings into a breaking-change-friendly
implementation package focused on reliability, truthful runtime state, and a
simpler browser deployment story. It intentionally removes or replaces stale API
shapes and ambiguous success semantics rather than preserving them for backward
compatibility.

The work should be executed in phases. Phase 1 hardens lifecycle, shutdown, and
recovery behavior. Phase 2 fixes settings truthfulness and schema boundaries.
Phase 3 cleans up snapshot serving and browser dependencies. The package
includes public API changes and internal interface changes, so it remains
approval-required before implementation.

## Implementation Tasks

### Phase 1 - Backend lifecycle, recovery, and shutdown reliability

1. Add an explicit backend lifecycle state model and make it the single source
   of truth for service health. Replace the current `_started`-only backend
   state with a small typed internal model covering `stopped`, `starting`,
   `running`, `recovering`, `degraded`, and `stopping`, plus `last_error`,
   `last_transition_at`, `consecutive_capture_failures`, and
   `last_recovery_attempt_at`. Expose backend-owned state transitions rather
   than inferring health from scattered flags.

2. Serialize lifecycle actions in the backend so start, stop, recovery, and
   shutdown cannot race each other. Introduce a backend-owned lifecycle lock or
   single-threaded lifecycle coordinator that owns camera connect, disconnect,
   capture-loop startup, capture-loop shutdown, and reconnect attempts. Capture
   thread failure signals, API-triggered recovery, bootstrap reads, and process
   shutdown must all flow through this serialized path.

3. Move runtime capture-failure handling out of the capture loop and into the
   backend recovery flow. When the capture loop crosses a fixed threshold of 5
   consecutive capture errors, it should notify the backend and stop trying to
   self-heal with local exponential sleeps. The backend should transition to
   `recovering`, stop capture, disconnect the camera, and attempt reconnects at
   fixed backoff intervals of 1s, 2s, and 5s. If those fail, transition to
   `degraded` and continue background retries every 30s until success. Recovery
   success must reload cached applied settings, re-queue any still-pending
   settings, restart capture, clear failure counters, and publish updated
   status.

4. Replace side-effecting `GET /api/connect` with explicit backend lifecycle
   endpoints. Add:
   - `POST /api/backend/start`
   - `POST /api/backend/recover`
   - `POST /api/backend/stop`

   `start` should bring a stopped backend into service and no-op when already
   active. `recover` should request an immediate recovery attempt when the
   backend is `recovering` or `degraded`, return a no-op response when already
   `running`, and reject invalid states such as `stopped`. `stop` should request
   backend shutdown and return the resulting or in-progress lifecycle state.
   Remove `GET /api/connect` from the plan rather than preserving it.

5. Make read endpoints side-effect free. `GET /api/bootstrap` and snapshot reads
   should not auto-start the backend. Bootstrap should report current status,
   capabilities, and last known applied settings without mutating backend state.
   Endpoints that require an active backend should return explicit `503` or
   state-aware errors rather than silently starting hardware on read access.

6. Fix shutdown behavior for long exposures by making capture interruption
   explicit and backend-owned. Extend the internal `CameraInterface` with
   `interrupt_capture() -> None`. Call it from `ContinuousCaptureLoop.stop()`
   before `join()`. Do not disconnect the camera until the worker has exited or
   a hard timeout has been reached. Use an exposure-aware join timeout of
   `max(0.5s, current_applied_exposure + 1s)` capped at 35s. The synthetic
   camera should replace raw `sleep()` with an interruptible wait so tests can
   prove stop behavior. The ZWO implementation should map `interrupt_capture()`
   to `stop_exposure()` and `stop_video_capture()`.

7. Define and implement an explicit stale-frame policy. Keep the last buffered
   frame in memory during `recovering` and `degraded` states for graceful
   degradation, but do not imply that it is live. Status and telemetry should
   expose `frame_timestamp`, `frame_age_seconds`, and an exposure-aware
   `frame_is_live` flag. `frame_is_live` should be false whenever backend state
   is not `running`, and otherwise should be computed from frame age using an
   initial threshold of `max(0.5s, current_applied_exposure + 1s)` capped at
   35s.

8. Update operator-facing logs and UI health state to match the new lifecycle.
   Log transitions into `starting`, `recovering`, `degraded`, `stopping`, and
   recovery success or exhaustion. Surface backend lifecycle state in bootstrap
   and telemetry so the UI can show explicit recovery/degraded messaging instead
   of generic transport errors.

### Phase 2 - Truthful settings semantics and schema hardening

9. Replace ad hoc settings parsing with typed Pydantic request and response
   models. `POST /api/settings` should validate with a schema model that forbids
   unknown keys, rejects malformed JSON types with FastAPI validation errors,
   rejects empty payloads with `400`, and keeps range validation
   capability-driven. Do not coerce arbitrary strings into numeric settings.

10. Change `POST /api/settings` to explicit accepted-then-applied semantics.
    The endpoint should queue only validated keys and return HTTP `202`
    `Accepted` with `queued_settings`, `queued_settings_version`,
    `applied_settings`, `applied_settings_version`, `settings_state`, and a
    timestamp. Do not return optimistic success payloads that imply settings are
    already in effect.

11. Add backend-owned queued-vs-applied settings tracking. The runtime should
    maintain distinct `queued_settings` and `applied_settings`, plus separate
    monotonically increasing `queued_settings_version` and
    `applied_settings_version`. When the capture thread successfully applies
    queued settings, it should publish an applied-settings event, persist the
    camera's actual current settings, advance `applied_settings_version`, and
    clear pending state. If application fails, it should leave persisted
    settings unchanged, retain or mark the queued state as failed, record the
    error in backend status, and publish failure telemetry that does not imply
    success.

12. Define pending-settings behavior across recovery and shutdown. Pending but
    unapplied settings should survive automatic recovery in memory and be
    re-applied after reconnect. Pending but unapplied settings should not be
    written to disk. A full explicit backend stop should discard still-pending
    settings after shutdown completes, while preserving the last successfully
    applied settings on disk.

13. Replace ambiguous telemetry settings fields with explicit applied/pending
    state. Telemetry payloads should include `backend_state`, `last_error`,
    `applied_settings`, `applied_settings_version`, `pending_settings`,
    `queued_settings_version`, `settings_state`, `frame_timestamp`,
    `frame_age_seconds`, `frame_is_live`, and `fps`. Remove the old generic
    `settings` field from the plan rather than keeping a misleading alias.

14. Replace the current loose backend status dict with a structured status
    response model. `GET /api/backend/status` should return typed sections for:
    - `backend`
    - `camera`
    - `capture`
    - `settings`
    - `timestamp`

    The status contract should clearly separate lifecycle state, camera
    connection state, frame liveness, queued/applied settings, and recovery
    metadata rather than merging arbitrary keys into one top-level dict.

### Phase 3 - Snapshot serving and offline-safe browser deployment

15. Make saved snapshots browser-fetchable through a dedicated served snapshot
    root rather than the entire repo assets tree. Standardize snapshot storage
    under `assets/snapshots/`, keep subdirectory validation and filename
    sanitization, and mount that snapshot root read-only for browser access.
    This avoids exposing unrelated fixture images such as synthetic camera test
    assets.

16. Simplify the snapshot API around fetchable URLs. `POST /api/snapshots`
    should return `filename`, `url`, `saved_at`, `frame_timestamp`,
    `frame_age_seconds`, and `applied_settings`. Remove legacy path-oriented
    response fields from the plan rather than preserving filesystem-like API
    details.

17. Clean up the browser dependency chain for offline-safe operation. Remove
    unused HTMX from the template. Eliminate CDN dependence by serving Alpine
    locally from `frontend/static/` and replacing the Tailwind CDN dependency
    with repo-local styles in `frontend/static/css/app.css`. Do not introduce a
    Node or Tailwind build step. Simplify templates and CSS as needed to remove
    reliance on runtime Tailwind generation.

18. Update the browser app to consume the new lifecycle and telemetry contracts.
    The frontend should use bootstrap and status data to distinguish `running`,
    `recovering`, `degraded`, and `stopped` states, reflect queued vs applied
    settings, and treat stale frames as degraded visibility rather than live
    data.

## Public API and Interface Changes

### Removed or replaced endpoints

- Remove `GET /api/connect`.
- Replace `GET /api/status` with `GET /api/backend/status`.

### Added lifecycle endpoints

- `POST /api/backend/start`
- `POST /api/backend/recover`
- `POST /api/backend/stop`

These endpoints return typed lifecycle responses and replace ambiguous
start-or-recover behavior.

### Changed read behavior

- `GET /api/bootstrap` becomes read-only and no longer auto-starts the backend.
- Snapshot creation no longer auto-starts the backend when it is stopped.

### Changed settings contract

- `POST /api/settings` becomes an asynchronous command endpoint that returns
  HTTP `202 Accepted`.
- Response fields distinguish queued settings from applied settings.
- Unknown keys are forbidden by schema.
- Generic immediate-success semantics are removed.

### Changed status and telemetry contracts

- `GET /api/backend/status` returns a typed structured payload instead of a
  merged backend/camera dict.
- Telemetry payloads expose backend lifecycle state, recovery metadata,
  frame-liveness data, and explicit pending/applied settings fields.
- The old generic telemetry `settings` field is removed.

### Changed snapshot contract

- `POST /api/snapshots` returns a browser-fetchable `url` and snapshot metadata.
- Legacy path-oriented response fields are removed from the plan.
- Browser-served files are limited to the dedicated snapshot root.

### Internal interface changes

- `CameraInterface` gains `interrupt_capture() -> None`.
- Backend service gains explicit lifecycle state tracking and serialized
  lifecycle control.
- Capture loop emits failure and settings-application events to the backend
  rather than owning recovery policy.

## Verification

- Add pytest coverage for lifecycle serialization so concurrent start, recover,
  stop, and shutdown requests cannot race into duplicate camera connect or
  disconnect operations.
- Add pytest coverage for repeated capture failures triggering backend-owned
  recovery, temporary failure recovering successfully, and persistent failure
  ending in `degraded`.
- Add pytest coverage for `POST /api/backend/start`,
  `POST /api/backend/recover`, and `POST /api/backend/stop` in valid and
  invalid lifecycle states.
- Add pytest coverage confirming `GET /api/bootstrap` is read-only and does not
  auto-start the backend.
- Add pytest coverage for stopping during a simulated long exposure and assert
  that `interrupt_capture()` is called, the worker exits before disconnect when
  possible, and timeout behavior is explicit when interruption fails.
- Add pytest coverage for stale-frame reporting: retained last frame during
  recovery, accurate `frame_age_seconds`, and `frame_is_live` false while
  `recovering` or `degraded`.
- Add pytest coverage for malformed settings payloads, unknown keys, empty
  payloads, out-of-range values, accepted queueing, successful apply, failed
  apply without persistence, and queued-settings survival across reconnect.
- Add pytest coverage confirming pending settings are discarded on full explicit
  stop but last applied settings remain persisted.
- Add pytest coverage for the new typed `GET /api/backend/status` response
  contract.
- Add pytest coverage for telemetry payload shape and semantics, including
  backend state, queued vs applied settings, and stale-frame flags.
- Add pytest coverage confirming snapshot responses include a fetchable `url`,
  snapshots are written under the dedicated snapshot root, and unrelated asset
  fixture directories are not browser-served.
- Add a frontend smoke test or template-level assertion that no third-party CDN
  URLs remain in rendered pages.
- Run the full repo quality gate after each implementation session:
  `uv run ruff check`, `uv run ruff format --check`, `uv run ty check`,
  `uv run deptry .`, and `uv run pytest`.

## Assumptions

- Breaking API changes are acceptable for this package, and the browser client
  can be updated in lockstep with backend changes.
- Automatic recovery remains the desired behavior even though it introduces a
  more explicit lifecycle model.
- Read endpoints should be side-effect free, even when that removes convenient
  legacy auto-start behavior.
- The system should prefer truthful state and graceful degradation over
  preserving optimistic or ambiguous success responses.
- Hardware-dependent recovery behavior will still be verified primarily through
  fake-camera seams in pytest, with manual ZWO sanity checks deferred to a later
  hardware session.
