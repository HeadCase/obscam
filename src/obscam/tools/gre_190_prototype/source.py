"""Deterministic full-resolution source for instrumentation validation."""

from __future__ import annotations

import asyncio
import io
import json
import struct
import time
from dataclasses import dataclass
from pathlib import Path

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
            exposure_end_unix_ns = time.time_ns()
            capture_complete_ns = time.perf_counter_ns()
            capture_complete_unix_ns = time.time_ns()
            encode_start_ns = time.perf_counter_ns()
            encode_start_unix_ns = time.time_ns()
            payload = await asyncio.to_thread(
                _encode_test_frame, generation, self._quality
            )
            encode_end_ns = time.perf_counter_ns()
            encode_end_unix_ns = time.time_ns()
            envelope = FrameEnvelope(
                generation=generation,
                exposure_end_ns=exposure_end_ns,
                exposure_end_unix_ns=exposure_end_unix_ns,
                capture_complete_ns=capture_complete_ns,
                capture_complete_unix_ns=capture_complete_unix_ns,
                encode_start_ns=encode_start_ns,
                encode_start_unix_ns=encode_start_unix_ns,
                encode_end_ns=encode_end_ns,
                encode_end_unix_ns=encode_end_unix_ns,
                encoded_bytes=len(payload),
            )
            await self._fanout.publish(EncodedFrame(envelope, payload))
            self.counters.produced += 1
            self.counters.encoded_bytes += len(payload)
            next_frame += self._period_s
            await asyncio.sleep(max(0, next_frame - time.perf_counter()))


class RustJpegSource:
    """Relay framed JPEG packets emitted by the native camera owner."""

    def __init__(
        self,
        fanout: LatestFrameFanout,
        *,
        exposure_us: int,
        gain: int,
        fps: int,
        binary: Path | None = None,
    ) -> None:
        self._fanout = fanout
        self._exposure_us = exposure_us
        self._gain = gain
        self._fps = fps
        self._binary = binary or (
            Path(__file__).with_name("rust_backend")
            / "target"
            / "debug"
            / "gre-190-rust-backend"
        )
        self.counters = SourceCounters()

    async def run(self) -> None:
        """Start the native owner and publish its compressed JPEG output."""
        process = await asyncio.create_subprocess_exec(
            self._binary,
            "dual-stream",
            "86400",
            str(self._exposure_us),
            str(self._gain),
            str(self._fps),
            stdout=asyncio.subprocess.PIPE,
        )
        assert process.stdout is not None
        try:
            while True:
                frame = await read_rust_jpeg_packet(process.stdout)
                await self._fanout.publish(frame)
                self.counters.produced += 1
                self.counters.encoded_bytes += len(frame.payload)
        finally:
            if process.returncode is None:
                process.terminate()
            await process.wait()


async def read_rust_jpeg_packet(reader: asyncio.StreamReader) -> EncodedFrame:
    """Read and validate one length-prefixed packet from the Rust backend."""
    magic = await reader.readexactly(4)
    if magic != b"GREJ":
        raise ValueError("invalid Rust JPEG packet magic")
    metadata_size = struct.unpack("!I", await reader.readexactly(4))[0]
    if metadata_size > 64 * 1024:
        raise ValueError("Rust JPEG metadata exceeds safety limit")
    metadata = json.loads(await reader.readexactly(metadata_size))
    payload_size = struct.unpack("!I", await reader.readexactly(4))[0]
    if payload_size > 16 * 1024 * 1024:
        raise ValueError("Rust JPEG payload exceeds safety limit")
    payload = await reader.readexactly(payload_size)
    envelope = FrameEnvelope.model_validate(metadata)
    if envelope.encoded_bytes != payload_size:
        raise ValueError("Rust JPEG payload length disagrees with metadata")
    if not payload.startswith(b"\xff\xd8") or not payload.endswith(b"\xff\xd9"):
        raise ValueError("Rust JPEG payload has invalid markers")
    return EncodedFrame(envelope, payload)


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
