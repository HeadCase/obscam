"""Validation helpers for camera settings requests."""

from typing import Any, cast

DEFAULT_EXPOSURE_RANGE_MS = (0.032, 30000.0)
DEFAULT_GAIN_RANGE = (0, 600)


def capability_range(
    capabilities: dict[str, object], key: str, default_min: float, default_max: float
) -> tuple[float, float]:
    """Extract a numeric min/max range from camera capabilities."""
    capability = capabilities.get(key)
    if isinstance(capability, dict):
        typed_capability = cast(dict[str, object], capability)
        min_val = typed_capability.get("min")
        max_val = typed_capability.get("max")
        if isinstance(min_val, (int, float)) and isinstance(max_val, (int, float)):
            return float(min_val), float(max_val)

    return float(default_min), float(default_max)


def validate_settings_payload(
    payload: dict[str, Any], capabilities: dict[str, object]
) -> dict[str, float | int]:
    """Return the subset of settings that is valid for the active camera."""
    exposure_min, exposure_max = capability_range(
        capabilities,
        "exposure_ms",
        DEFAULT_EXPOSURE_RANGE_MS[0],
        DEFAULT_EXPOSURE_RANGE_MS[1],
    )
    gain_min, gain_max = capability_range(
        capabilities,
        "gain",
        DEFAULT_GAIN_RANGE[0],
        DEFAULT_GAIN_RANGE[1],
    )

    valid_settings: dict[str, float | int] = {}
    if "exposure_ms" in payload:
        exposure_ms = float(payload["exposure_ms"])
        if exposure_min <= exposure_ms <= exposure_max:
            valid_settings["exposure_ms"] = exposure_ms

    if "gain" in payload:
        gain = int(payload["gain"])
        if gain_min <= gain <= gain_max:
            valid_settings["gain"] = gain

    return valid_settings
