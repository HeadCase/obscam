"""Unit coverage for GRE-183 distribution calculations."""

import pytest

from obscam.tools.gre_183_prototype.statistics import (
    percentile,
    summarize_timings,
)


def test_percentile_uses_linear_interpolation() -> None:
    values = [1.0, 2.0, 3.0, 4.0]

    assert percentile(values, 0.50) == 2.5
    assert percentile(values, 0.95) == pytest.approx(3.85)
    assert percentile(values, 0.99) == pytest.approx(3.97)


def test_summarize_timings_handles_empty_input() -> None:
    summary = summarize_timings([])

    assert summary.count == 0
    assert summary.p50_ms is None
    assert summary.maximum_ms is None


def test_summarize_timings_reports_required_distribution() -> None:
    summary = summarize_timings([4.0, 1.0, 3.0, 2.0])

    assert summary.count == 4
    assert summary.minimum_ms == 1.0
    assert summary.mean_ms == 2.5
    assert summary.p50_ms == 2.5
    assert summary.p95_ms == pytest.approx(3.85)
    assert summary.p99_ms == pytest.approx(3.97)
    assert summary.maximum_ms == 4.0
