"""Measurement contract for the GRE-190 throwaway benchmark."""

from __future__ import annotations

from enum import StrEnum
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator

SCHEMA_VERSION = 1
FULL_FRAME_WIDTH = 1920
FULL_FRAME_HEIGHT = 1080


class DeliveryPath(StrEnum):
    """Delivery paths compared by GRE-190."""

    JPEG_WEBSOCKET = "jpeg-websocket"
    MULTIPART_MJPEG = "multipart-mjpeg"
    H264_WEBRTC = "h264-webrtc"
    H264_MSE = "h264-mse"


class FrameEnvelope(BaseModel):
    """Server-side timings attached to one encoded frame."""

    model_config = ConfigDict(frozen=True)

    schema_version: Literal[1] = SCHEMA_VERSION
    generation: int = Field(ge=1)
    exposure_end_ns: int = Field(ge=0)
    capture_complete_ns: int = Field(ge=0)
    encode_start_ns: int = Field(ge=0)
    encode_end_ns: int = Field(ge=0)
    width: Literal[1920] = FULL_FRAME_WIDTH
    height: Literal[1080] = FULL_FRAME_HEIGHT
    encoded_bytes: int = Field(gt=0)

    @model_validator(mode="after")
    def validate_timeline(self) -> FrameEnvelope:
        """Reject impossible server-side timing sequences."""
        if not (
            self.exposure_end_ns
            <= self.capture_complete_ns
            <= self.encode_start_ns
            <= self.encode_end_ns
        ):
            raise ValueError("frame timestamps are not monotonic")
        return self


class BrowserPresentation(BaseModel):
    """One frame-presentation event reported by a real browser."""

    schema_version: Literal[1] = SCHEMA_VERSION
    client_id: str = Field(min_length=1, max_length=128)
    path: DeliveryPath
    generation: int = Field(ge=1)
    browser_receive_ms: float = Field(ge=0)
    decode_complete_ms: float = Field(ge=0)
    visible_ms: float = Field(ge=0)
    server_encode_end_ns: int = Field(ge=0)
    clock_offset_ms: float
    clock_uncertainty_ms: float = Field(ge=0)
    visibility_state: str

    @model_validator(mode="after")
    def validate_browser_timeline(self) -> BrowserPresentation:
        """Require receive, decode, and visible events to be ordered."""
        if not (self.browser_receive_ms <= self.decode_complete_ms <= self.visible_ms):
            raise ValueError("browser presentation timestamps are not monotonic")
        return self


class PathCounters(BaseModel):
    """Latest-frame delivery counters for one path."""

    path: DeliveryPath
    produced: int = Field(ge=0)
    connected_clients: int = Field(ge=0)
    delivered: int = Field(ge=0)
    generation_replacements: int = Field(ge=0)
    encoded_bytes: int = Field(ge=0)
