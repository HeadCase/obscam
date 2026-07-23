"""Distribution calculations shared by GRE-183 Python tooling."""

from __future__ import annotations

import math
import statistics
from collections.abc import Sequence

from obscam.tools.gre_183_prototype.contract import TimingSummary


def percentile(sorted_values: Sequence[float], quantile: float) -> float:
    """Return a linearly interpolated percentile from sorted values."""
    if not sorted_values:
        raise ValueError("cannot calculate a percentile from no values")
    if not 0.0 <= quantile <= 1.0:
        raise ValueError("quantile must be between zero and one")

    position = (len(sorted_values) - 1) * quantile
    lower_index = math.floor(position)
    upper_index = math.ceil(position)
    if lower_index == upper_index:
        return sorted_values[lower_index]
    lower = sorted_values[lower_index]
    upper = sorted_values[upper_index]
    return lower + ((upper - lower) * (position - lower_index))


def summarize_timings(values: Sequence[float]) -> TimingSummary:
    """Summarize a complete set of millisecond timings."""
    if not values:
        return TimingSummary(
            count=0,
            minimum_ms=None,
            mean_ms=None,
            p50_ms=None,
            p95_ms=None,
            p99_ms=None,
            maximum_ms=None,
        )

    ordered = sorted(values)
    return TimingSummary(
        count=len(ordered),
        minimum_ms=ordered[0],
        mean_ms=statistics.fmean(ordered),
        p50_ms=percentile(ordered, 0.50),
        p95_ms=percentile(ordered, 0.95),
        p99_ms=percentile(ordered, 0.99),
        maximum_ms=ordered[-1],
    )
