"""Local contract and isolation checks for the GRE-190 prototype."""

from __future__ import annotations

import asyncio
from pathlib import Path

import pytest
from pydantic import ValidationError

from obscam.tools.gre_190_prototype.contract import (
    BrowserPresentation,
    BrowserRunReport,
    ClockSample,
    DeliveryPath,
    FrameEnvelope,
)
from obscam.tools.gre_190_prototype.hardware_h264 import HardwareH264Scenario
from obscam.tools.gre_190_prototype.latest import EncodedFrame, LatestFrameFanout


def envelope(generation: int) -> FrameEnvelope:
    """Create a valid compact test envelope."""
    return FrameEnvelope(
        generation=generation,
        exposure_end_ns=1,
        exposure_end_unix_ns=1,
        capture_complete_ns=2,
        capture_complete_unix_ns=2,
        encode_start_ns=3,
        encode_start_unix_ns=3,
        encode_end_ns=4,
        encode_end_unix_ns=4,
        encoded_bytes=1,
    )


def test_frame_envelope_rejects_impossible_timeline() -> None:
    """Invalid timestamps must not enter benchmark evidence."""
    with pytest.raises(ValidationError):
        FrameEnvelope(
            generation=1,
            exposure_end_ns=2,
            exposure_end_unix_ns=1,
            capture_complete_ns=1,
            capture_complete_unix_ns=2,
            encode_start_ns=3,
            encode_start_unix_ns=3,
            encode_end_ns=4,
            encode_end_unix_ns=4,
            encoded_bytes=1,
        )


def test_hardware_h264_command_forbids_software_fallback(tmp_path: Path) -> None:
    """The gate must explicitly select the Pi V4L2 M2M encoder."""
    scenario = HardwareH264Scenario(duration_s=2)
    arguments = scenario.ffmpeg_arguments(tmp_path / "probe.h264")
    assert scenario.frames == 20
    assert arguments[arguments.index("-c:v") + 1] == "h264_v4l2m2m"
    assert "libx264" not in arguments


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


def test_browser_run_rejects_mismatched_presentation_count() -> None:
    """A partial telemetry upload cannot masquerade as a complete run."""
    event = BrowserPresentation(
        client_id="safari-1",
        path=DeliveryPath.JPEG_WEBSOCKET,
        generation=1,
        browser_receive_ms=1,
        decode_complete_ms=2,
        visible_ms=3,
        server_encode_end_ns=1,
        clock_offset_ms=0,
        clock_uncertainty_ms=1,
        visibility_state="visible",
    )
    with pytest.raises(ValidationError, match="presentation count"):
        BrowserRunReport(
            run_id="wireguard-1",
            client_id="safari-1",
            path=DeliveryPath.JPEG_WEBSOCKET,
            user_agent="Safari",
            started_unix_ms=1,
            completed_unix_ms=2,
            requested_duration_s=60,
            received_frames=2,
            unique_presented_frames=2,
            skipped_generations=0,
            reconnects=1,
            hidden_events=0,
            clock=ClockSample(
                browser_send_unix_ms=1,
                server_receive_unix_ns=1,
                server_send_unix_ns=2,
                browser_receive_unix_ms=2,
                offset_ms=0,
                uncertainty_ms=0.5,
            ),
            presentations=[event],
        )
