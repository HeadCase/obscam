# AGENTS.md - Working Rules for obscam

## Testing and Verification

- Tests must be written with `pytest`.

## Quality Gate

- After any code change, run all applicable project code quality checks.
- The default quality checks for this repo are:
  - `uv run ruff check`
  - `uv run ruff format --check`
  - `uv run ty check`
  - `uv run deptry .`
  - `uv run pytest`

## Build and Code Conventions

- Use modern type annotations such as `dict[str, int]` and `list[str]`.
- Use package imports such as `from obscam.foo import Foo`.
- Use `loguru` for logging.
- Build system: `uv_build`

## References

### ZWO SDKs

- https://zwoastro.yuque.com/olyczd/sfwyw6/kpde2odaw3h4ekix
- https://github.com/python-zwoasi/python-zwoasi
- https://raw.githubusercontent.com/python-zwoasi/python-zwoasi/refs/heads/master/zwoasi/examples/zwoasi_demo.py
