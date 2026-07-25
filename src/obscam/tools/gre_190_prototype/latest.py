"""Bounded latest-frame fan-out used by the GRE-190 probes."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass

from obscam.tools.gre_190_prototype.contract import FrameEnvelope


@dataclass(frozen=True, slots=True)
class EncodedFrame:
    """An immutable encoded frame and its measurement envelope."""

    envelope: FrameEnvelope
    payload: bytes


class LatestFrameFanout:
    """Keep exactly one shared current frame and isolate every reader."""

    def __init__(self) -> None:
        self._condition = asyncio.Condition()
        self._frame: EncodedFrame | None = None

    async def publish(self, frame: EncodedFrame) -> None:
        """Replace the current generation and wake all waiting readers."""
        async with self._condition:
            if self._frame is not None and frame.envelope.generation <= (
                self._frame.envelope.generation
            ):
                raise ValueError("frame generations must increase")
            self._frame = frame
            self._condition.notify_all()

    async def wait_after(self, generation: int) -> EncodedFrame:
        """Return the first current frame newer than ``generation``."""
        async with self._condition:
            await self._condition.wait_for(
                lambda: (
                    self._frame is not None
                    and self._frame.envelope.generation > generation
                )
            )
            assert self._frame is not None
            return self._frame
