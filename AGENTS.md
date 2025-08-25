# AGENTS.md - Coding Guidelines for obscam

## Build/Test Commands

- **Install dependencies**: `uv add` or `uv sync`
- **Run main application**: `uv run obscam`
- **Run python code with development environment using**: uv run
- **Tests should be written with pytest**
- _Use package name for imports_: `from obscam.foo import Foo`
- _Use loguru for logging_

## Code Style & Conventions

- **Python version**: >=3.11 (specified in pyproject.toml)
- **Type hints**: Use modern Python type annotations (e.g., `dict[str, int]`, `list[str]`)
- **Imports**: Group stdlib, third-party, local imports with blank lines between groups
- **Variables**: snake_case for variables/functions, UPPER_CASE for constants
- **Error handling**: Use specific exceptions, try/except blocks with minimal scope
- **Classes**: CamelCase class names, use type annotations for attributes

## Dependencies

- Core: FastAPI, Flask
- Dev: ipython for development/debugging
- Build system: uv_build

## General Guidelines

- This application's purpose to provide monitoring (CCTV-style) for my remote
  astrophotography observatory
- I access my observatory and all its functions remotely via Wireguard
- The camera used for monitoring is pointed at my telescope, with a view of the
  observatory roof which rolls on and off at my instruction
- Monitoring my telescope during slewing actions is particularly important, and
  frames need to be updated every 200-500 milliseconds to make this worthwhile
- Sometimes I need a long exposure (1-10 seconds) when it's really dark or the
  roof is closed

## Pi-Specific Design Principles

1. REUSE over CREATE - Use existing Flask/FastAPI threads
2. MEMORY over DISK - Keep state in RAM, not databases
3. CLIENT over SERVER - Push complexity to desktop browsers
4. SIMPLE over FEATURE-RICH - Observatory needs reliability, not features
5. GRACEFUL DEGRADATION - Lower quality beats service failure

<!-- ## Architecture Notes -->
<!-- - Camera control via ZWO ASI SDK bindings -->
<!-- - Dual capture modes: video (short exposures) vs snapshot (long exposures)  -->
<!-- - Buffer flushing strategy to prevent stale frames -->
<!-- - Thread-safe frame grabbing with latest-frame semantics -->
