# AGENTS.md - Working Rules for obscam

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

1. REUSE over CREATE - Reuse existing Flask/FastAPI threads and flows where practical.
2. MEMORY over DISK - Keep runtime state in RAM, not persistent storage.
3. CLIENT over SERVER - Push interaction and presentation complexity to the desktop browser.
4. SIMPLE over FEATURE-RICH - Favor reliability over optional capability.
5. GRACEFUL DEGRADATION - Lower quality is preferable to service failure.

## Change Boundaries

- Include adjacent cleanup only when it materially improves the area being changed.
- Preserve backward compatibility for internal interfaces unless preserving it would materially expand scope.
- For public or user-facing APIs, propose breaking changes only when clearly justified.
- Avoid adding server-side complexity that could reasonably live in the client.
- New dependencies are acceptable only when they significantly reduce code complexity.
- Prefer thin wrappers over vendor camera SDKs.
- Choose the simplest design that works for current hardware rather than building generalized abstraction early.

## Testing and Verification

- Tests must be written with `pytest`.
- Add tests whenever behavior changes.
- Prefer unit tests plus integration seams where possible, especially around hardware-dependent paths.
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
  - `uv run ruff check`
  - `uv run ruff format --check`
  - `uv run ty check`
  - `uv run deptry .`
  - `uv run pytest`
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
- Low-latency monitoring mode should aim to support roughly 10ms exposures and near-honest client-visible framerates when hardware, sensor mode, and lighting permit.
- Long exposures may still be needed in very dark conditions or when the roof is closed.

## Build and Code Conventions

- Python version: `>=3.11`
- Use modern type annotations such as `dict[str, int]` and `list[str]`.
- Use package imports such as `from obscam.foo import Foo`.
- Use `loguru` for logging.
- Build system: `uv_build`

## Python Development Rules

- Treat these rules as strong defaults. Deviate only when there is a clear, local justification.

### Typing

- Require type annotations everywhere in new or modified code, including private helpers.
- Add explicit return types on all functions and methods.
- Avoid `Any` by default. Use it only with explicit justification.
- Prefer precise types over broad unions or loosely typed containers.
- Wrap or isolate untyped third-party APIs before they reach core application logic where practical.

### Data Models

- Prefer `pydantic` for external, configuration, and shared structured models.
- Use lighter typed classes or dataclasses for internal hot-path state where validation and serialization are not needed.
- Avoid passing ad hoc dictionaries across module boundaries. Define typed models instead.
- Prefer `pydantic` settings/models for configuration and environment-derived settings.
- Favor explicit, named models over loosely structured payloads.

### Validation

- Validate aggressively at system boundaries, state transitions, and integration seams.
- Avoid repeated runtime validation in hot internal paths unless it protects a critical invariant.
- Coerce external input when it is clearly safe and predictable; otherwise fail with clear errors.
- Validate and normalize hardware SDK and vendor responses as early as practical.
- Do not let raw vendor-specific payloads spread through the codebase unless there is a strong performance reason.

### Design

- Prefer simple concrete functions and classes by default.
- Use `Protocol` when abstraction is needed.
- Use generics sparingly and only when they clearly improve correctness or API clarity.
- Prefer synchronous code unless async is clearly necessary for correctness or performance.
- When code becomes complex, first improve data models and simplify control flow before adding layers.
- Prefer clearer models and more direct flow over additional abstraction.

### Errors

- Use specific custom exceptions at system and integration boundaries.
- Prefer standard library exceptions internally unless a custom exception materially improves handling.
- Keep exception scope tight and avoid broad catch-and-continue patterns.

### Testability

- Use dependency injection where it materially improves testability or isolation.
- Prefer real components or high-fidelity seams where possible rather than heavy mock-driven tests.
- Minimize mocks, especially for internal logic, unless they are the clearest way to isolate an external dependency.

### Documentation

- Add docstrings to public modules, classes, and functions.
- Keep docstrings concise and focused on behavior, inputs, outputs, and non-obvious constraints.
- Prefer readable code and well-named types over verbose inline explanation.

## References

### ZWO SDKs
- https://zwoastro.yuque.com/olyczd/sfwyw6/kpde2odaw3h4ekix
- https://github.com/python-zwoasi/python-zwoasi
- https://raw.githubusercontent.com/python-zwoasi/python-zwoasi/refs/heads/master/zwoasi/examples/zwoasi_demo.py
