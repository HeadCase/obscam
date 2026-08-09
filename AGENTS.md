# AGENTS.md - Working Rules for obscam

## Agent skills

### Issue tracker

Issues are tracked in the Greg Headley team's ObsCam project in Linear. See
`docs/agents/issue-tracker.md`.

### Triage labels

The tracker uses the canonical five-role triage vocabulary. See
`docs/agents/triage-labels.md`.

### Domain docs

This is a single-context repository. See `docs/agents/domain.md`.

### Architecture authority

Linear issue GRE-179 and its accepted child decisions are the architectural
system of record. See `docs/agents/architecture.md` for the local projection.
Historical evidence and deleted implementations remain in Git history, not the
active tree. Do not inspect or recover them unless the user explicitly requests
historical investigation.

### Qualified appliance releases

Use `docs/agents/qualified-releases.md` to install, activate, roll back, and
diagnose the managed appliance. Do not improvise a custom media stack or allow
browser testing to inspect or advertise `wg1`.

## Decision Policy

- Always propose before implementing.
- Do not implement when requirements are ambiguous. Stop and ask.
- Treat any schema, API, or interface change as approval-required.
- When presenting a proposal, include:
  - `Summary`
  - `Changes`
  - `Verification`
  - `Assumptions`
- In proposals, include key tradeoffs and affected components.
- When a design choice exists, present 2-3 options with a recommendation.
- If a requested approach conflicts with project principles, recommend an alternative and wait for confirmation.

## Git Workflow

- Never commit directly to `develop` or `main`.
- Before changing files, create or switch to a feature branch. Agent-created
  branches use the `codex/` prefix unless the user requests another name.
- All changes enter `develop` through a pull request from the feature branch.
- If work begins while `develop` or `main` is checked out, create the feature
  branch before staging or committing any change.
- GitHub CLI credentials are available in the host execution context, not the
  command sandbox. Run `gh` and GitHub network operations outside the sandbox.
  A sandboxed authentication failure is not authoritative: rerun
  `gh auth status` outside the sandbox before diagnosing credentials. Never run
  `gh auth login` or `gh auth refresh` solely because a sandboxed check failed.
- All Git fetch, pull, and push operations must use the repository's configured
  SSH remote. Never rewrite GitHub remotes, substitute HTTPS URLs, or use
  token-authenticated HTTPS as a fallback. Never run `gh auth setup-git`. If SSH
  authentication fails outside the sandbox, stop and report the failure; do not
  change authentication or transport configuration. `gh` may still be used
  outside the sandbox for GitHub API operations such as creating pull requests,
  but Git object transfer remains SSH-only.

## Response Style

- Be concise but complete.
- Default response structure:
  - `Summary`
  - `Changes`
  - `Verification`
- Add a dedicated `Assumptions` section whenever assumptions affect design, behavior, or validation.
- For code review:
  - list findings first
  - order findings by severity
  - include file/line references
  - if there are no findings, say so explicitly and include residual risks and testing gaps

## Engineering Priorities

- Optimize first for reliability.
- Prefer responsiveness over image quality when those goals conflict.
- Optimize for Raspberry Pi constraints:
  - CPU efficiency
  - memory efficiency
  - low disk I/O
- Keep the server minimal and push complexity to the browser where practical.
- Prefer cleaner refactors when they improve future work.
- Aggressively remove unnecessary indirection, dead paths, and abstraction that do not materially help the system.
- Prefer performance-path simplification and removal of abstraction over architectural expansion.

## Architecture Rules

1. GREENFIELD over LEGACY - Implement the accepted Rust architecture without
   recovering superseded application or prototype code.
2. MEMORY over DISK - Keep runtime state in RAM, not persistent storage.
3. CLIENT over SERVER - Push interaction and presentation complexity to the browser.
4. SIMPLE over FEATURE-RICH - Favor reliability over optional capability.
5. GRACEFUL DEGRADATION - Lower quality is preferable to service failure.
6. ONE MEDIA PATH - Use only shared hardware H.264 through MediaMTX WHEP/WebRTC.

Prohibited unless a later approved Linear decision changes the map:

- a Python application server
- JPEG, MJPEG, or a secondary media-delivery path
- server-side recording, image history, or runtime-state persistence
- image rotation or silent spatial-resolution reduction
- importing implementation from Git history

## Change Boundaries

- Include adjacent cleanup only when it materially improves the area being changed.
- Preserve backward compatibility for internal interfaces unless preserving it would materially expand scope.
- For public or user-facing APIs, propose breaking changes only when clearly justified.
- Avoid adding server-side complexity that could reasonably live in the client.
- New dependencies are acceptable only when they significantly reduce code complexity.
- Prefer thin wrappers over vendor camera SDKs.
- Choose the simplest design that works for current hardware rather than building generalized abstraction early.

## Testing and Verification

- Add tests whenever behavior changes.
- For graphical browser verification, follow
  `docs/agents/browser-testing.md`. A Playwright MCP runs on the operator's Mac
  and is available to agents through an SSH tunnel; the Mac browser reaches the
  Pi service over WireGuard at `10.44.0.1` or, as a LAN fallback only, at
  `192.168.1.200`. Never infer that browser testing is unavailable from the
  absence of a browser executable on the headless Pi.
- Never use or probe `wg1` (`192.168.4.9`) for browser testing. It is unrelated
  infrastructure used to write astrophotography images to the operator's NAS
  and must not be inspected, tested, reconfigured, or treated as a fallback.
- Prefer the deployed-stack acceptance seam: exercise the real Rust service,
  FFmpeg, and MediaMTX through browser-facing contracts, substituting only
  unavailable hardware edges.
- Use focused Rust unit tests for state machines and algorithms, not as a
  substitute for deployed-stack verification.
- Before calling work complete, perform:
  - relevant automated tests
  - sanity checks where possible
  - explicit edge-case review
- If verification is blocked by environment or unavailable hardware, stop and report the limitation. Do not guess.
- Sanity checks should use available local signals such as:
  - startup behavior
  - logs
  - health endpoints
  - non-hardware execution paths

## Quality Gate

- After any code change, run all applicable project code quality checks before treating the work as complete.
- The default quality checks for this repo are:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `cargo test --workspace --all-targets --all-features`
  - `cargo deny check`
  - `cargo machete`
- Browser checks become required when browser code exists.
- Resolve failures from these checks as part of the change when they are in scope.
- If a check is unavailable due to environment setup or missing dependencies, report that explicitly.
- Do not treat work as complete if any required check fails.
- In the final response, report which checks were run, which passed, and which were blocked or not run.

## Edge Cases To Review

When relevant, explicitly consider:
- camera disconnect and reconnect behavior
- stale frames
- latency spikes
- multi-client session conflicts
- degraded network or VPN interruptions

## Project Context

- obscam is a CCTV-style monitoring system for a remote astrophotography observatory.
- Monitoring telescope slews is operationally important.
- Low-latency monitoring mode supports a 50 ms exposure floor and should retain
  a near-honest 20 fps client-visible ceiling when hardware, sensor mode, and
  lighting permit.
- Long exposures may still be needed in very dark conditions or when the roof is closed.

## Build and Code Conventions

- The application and camera owner are Rust.
- Use the repository's pinned stable Rust toolchain once the workspace exists.
- Deny Clippy warnings across the whole workspace, all targets, and all features.
- Prefer concrete types and direct control flow over generalized abstraction.
- Isolate unsafe FFI at the ZWO SDK boundary and document its safety invariants.
- Validate external inputs and normalize vendor responses at integration boundaries.
- Keep hot-path allocations, copies, locks, and disk I/O explicit and minimal.
- Use structured tracing for operational logs and telemetry.
- Document public APIs and non-obvious safety or performance constraints.

## References

### ZWO SDKs
- https://zwoastro.yuque.com/olyczd/sfwyw6/kpde2odaw3h4ekix
