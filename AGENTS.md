# AGENTS.md - Coding Guidelines for obscam

## Build/Test Commands
- **Install dependencies**: `uv install` or `uv sync`
- **Run main application**: `uv run obscam` 
- **Run arbitrary python scripts**: uv run example.py
- **No test framework configured** - check with maintainer for test setup

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

<!-- ## Architecture Notes -->
<!-- - Camera control via ZWO ASI SDK bindings -->
<!-- - Dual capture modes: video (short exposures) vs snapshot (long exposures)  -->
<!-- - Buffer flushing strategy to prevent stale frames -->
<!-- - Thread-safe frame grabbing with latest-frame semantics -->
