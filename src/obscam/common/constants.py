from pathlib import Path


def _find_project_root() -> Path:
    current = Path(__file__).resolve()
    for parent in current.parents:
        if (parent / "pyproject.toml").exists():
            return parent
    raise RuntimeError("Could not locate project root from obscam.common.constants")


PROJECT_ROOT = _find_project_root()
DEFAULT_CACHE_DIR = Path("/tmp")
ASSETS_DIR = PROJECT_ROOT / "assets"
FRONTEND_DIR = PROJECT_ROOT / "frontend"
STATIC_DIR = FRONTEND_DIR / "static"
TEMPLATE_DIR = FRONTEND_DIR / "templates"
LOG_DIR = PROJECT_ROOT / "logs"
