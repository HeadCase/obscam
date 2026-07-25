"""Deterministic full-resolution source for instrumentation validation."""

from __future__ import annotations

import asyncio
import io
import time
from dataclasses import dataclass

from PIL import Image, ImageDraw

from obscam.tools.gre_190_prototype.contract import (
    FULL_FRAME_HEIGHT,
    FULL_FRAME_WIDTH,
    FrameEnvelope,
)
from obscam.tools.gre_190_prototype.latest import EncodedFrame, LatestFrameFanout


@dataclass(slots=True)
class SourceCounters:
    """Mutable counters surfaced by the throwaway server."""

    produced: int = 0
    encoded_bytes: int = 0


class GeneratedJpegSource:
    """Generate identifiable 1920x1080 frames at a bounded cadence."""

    def __init__(
        self,
        fanout: LatestFrameFanout,
        *,
        fps: float,
        quality: int,
    ) -> None:
        self._fanout = fanout
        self._period_s = 1 / fps
        self._quality = quality
        self.counters = SourceCounters()

    async def run(self) -> None:
        """Publish frames forever until the owning task is cancelled."""
        generation = 0
        next_frame = time.perf_counter()
        while True:
            generation += 1
            exposure_end_ns = time.perf_counter_ns()
            capture_complete_ns = time.perf_counter_ns()
            encode_start_ns = time.perf_counter_ns()
            payload = await asyncio.to_thread(
                _encode_test_frame, generation, self._quality
            )
            encode_end_ns = time.perf_counter_ns()
            envelope = FrameEnvelope(
                generation=generation,
                exposure_end_ns=exposure_end_ns,
                capture_complete_ns=capture_complete_ns,
                encode_start_ns=encode_start_ns,
                encode_end_ns=encode_end_ns,
                encoded_bytes=len(payload),
            )
            await self._fanout.publish(EncodedFrame(envelope, payload))
            self.counters.produced += 1
            self.counters.encoded_bytes += len(payload)
            next_frame += self._period_s
            await asyncio.sleep(max(0, next_frame - time.perf_counter()))


def _encode_test_frame(generation: int, quality: int) -> bytes:
    """Render a high-contrast frame ID and return one JPEG payload."""
    shade = generation % 256
    image = Image.new("L", (FULL_FRAME_WIDTH, FULL_FRAME_HEIGHT), shade)
    draw = ImageDraw.Draw(image)
    text = f"GRE-190 GENERATION {generation:012d}"
    draw.rectangle((32, 32, 760, 120), fill=255 - shade)
    draw.text((48, 56), text, fill=shade)
    output = io.BytesIO()
    image.save(output, format="JPEG", quality=quality, optimize=False)
    return output.getvalue()
