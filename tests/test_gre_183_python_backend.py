"""Safety coverage for GRE-183 camera selection."""

import pytest

from obscam.tools.gre_183_prototype.python_backend import required_camera_index


def test_required_camera_index_does_not_probe_a_different_model() -> None:
    camera_names = ["ZWO ASI178MC", "ZWO ASI662MC"]

    assert required_camera_index(camera_names, "ASI662MC") == 1


def test_required_camera_index_requires_unique_model_identity() -> None:
    with pytest.raises(RuntimeError, match="not a unique hardware identity"):
        required_camera_index(
            ["ZWO ASI662MC", "ZWO ASI662MC"],
            "ASI662MC",
        )
