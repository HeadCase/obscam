"""Local contract and isolation checks for the GRE-190 prototype."""

from __future__ import annotations

import asyncio

import pytest
from pydantic import ValidationError

from obscam.tools.gre_190_prototype.contract import FrameEnvelope
from obscam.tools.gre_190_prototype.latest import EncodedFrame, LatestFrameFanout


def envelope(generation: int) -> FrameEnvelope:
    """Create a valid compact test envelope."""
    return FrameEnvelope(
        generation=generation,
        exposure_end_ns=1,
        capture_complete_ns=2,
        encode_start_ns=3,
        encode_end_ns=4,
        encoded_bytes=1,
    )


def test_frame_envelope_rejects_impossible_timeline() -> None:
    """Invalid timestamps must not enter benchmark evidence."""
    with pytest.raises(ValidationError):
        FrameEnvelope(
            generation=1,
            exposure_end_ns=2,
            capture_complete_ns=1,
            encode_start_ns=3,
            encode_end_ns=4,
            encoded_bytes=1,
        )


def test_fanout_returns_latest_generation_without_queueing() -> None:
    """A lagging reader skips replaced frames and receives the latest one."""

    async def scenario() -> None:
        fanout = LatestFrameFanout()
        await fanout.publish(EncodedFrame(envelope(1), b"1"))
        await fanout.publish(EncodedFrame(envelope(2), b"2"))
        frame = await fanout.wait_after(0)
        assert frame.envelope.generation == 2
        assert frame.payload == b"2"

    asyncio.run(scenario())


def test_fanout_rejects_non_increasing_generation() -> None:
    """Producer bugs cannot silently corrupt unique-frame measurements."""

    async def scenario() -> None:
        fanout = LatestFrameFanout()
        await fanout.publish(EncodedFrame(envelope(1), b"1"))
        with pytest.raises(ValueError, match="generations must increase"):
            await fanout.publish(EncodedFrame(envelope(1), b"again"))

    asyncio.run(scenario())
