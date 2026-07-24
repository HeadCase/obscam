"""Shared data contract for the GRE-183 benchmark runners."""

from __future__ import annotations

from enum import StrEnum
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator

SCHEMA_VERSION = 1
FULL_FRAME_WIDTH = 1920
FULL_FRAME_HEIGHT = 1080


class ImageFormat(StrEnum):
    """SDK image formats exercised by GRE-183."""

    Y8 = "Y8"
    RAW8 = "RAW8"
    RGB24 = "RGB24"

    @property
    def sdk_value(self) -> int:
        """Return the corresponding ``ASI_IMG_TYPE`` integer."""
        return {
            ImageFormat.RAW8: 0,
            ImageFormat.RGB24: 1,
            ImageFormat.Y8: 3,
        }[self]

    @property
    def bytes_per_pixel(self) -> int:
        """Return the SDK output bytes per pixel for this format."""
        return 3 if self is ImageFormat.RGB24 else 1


class RunnerKind(StrEnum):
    """Capture implementations compared by the benchmark."""

    NATIVE_C = "native-c"
    PYTHON_ZWOASI = "python-zwoasi"
    RUST_PIPELINE = "rust-pipeline"


class AcquisitionMode(StrEnum):
    """Native SDK acquisition regimes compared by GRE-183."""

    VIDEO = "video"
    SNAPSHOT = "snapshot"


class Scenario(BaseModel):
    """One directly comparable full-frame benchmark scenario."""

    model_config = ConfigDict(frozen=True)

    image_format: ImageFormat
    exposure_us: int = Field(ge=1)
    gain: int = Field(ge=0)
    high_speed: int = Field(ge=0, le=1)
    bandwidth: int = Field(ge=0)
    duration_s: float = Field(gt=0)
    warmup_frames: int = Field(default=5, ge=0)
    timeout_ms: int = Field(ge=1)
    acquisition_mode: AcquisitionMode = AcquisitionMode.VIDEO
    transition_from_exposure_us: int | None = Field(default=None, ge=1)
    transition_after_ms: float | None = Field(default=None, gt=0)
    width: Literal[1920] = FULL_FRAME_WIDTH
    height: Literal[1080] = FULL_FRAME_HEIGHT

    @model_validator(mode="after")
    def require_upward_exposure_transition(self) -> Scenario:
        """Keep transition classification unambiguous for queued video frames."""
        if (
            self.transition_from_exposure_us is not None
            and self.acquisition_mode is AcquisitionMode.VIDEO
            and self.transition_after_ms is None
            and self.transition_from_exposure_us >= self.exposure_us
        ):
            raise ValueError(
                "transition_from_exposure_us must be lower than exposure_us"
            )
        if (
            self.transition_after_ms is not None
            and self.transition_from_exposure_us is None
        ):
            raise ValueError("transition_after_ms requires an exposure transition")
        if (
            self.acquisition_mode is AcquisitionMode.SNAPSHOT
            and self.transition_from_exposure_us is not None
            and self.transition_from_exposure_us == self.exposure_us
        ):
            raise ValueError("snapshot transition exposures must differ")
        return self

    @property
    def buffer_bytes(self) -> int:
        """Return the exact caller-owned SDK buffer size."""
        return self.width * self.height * self.image_format.bytes_per_pixel

    def runner_arguments(self) -> list[str]:
        """Render the common command-line arguments accepted by every runner."""
        arguments = [
            "--format",
            self.image_format.value,
            "--exposure-us",
            str(self.exposure_us),
            "--gain",
            str(self.gain),
            "--high-speed",
            str(self.high_speed),
            "--bandwidth",
            str(self.bandwidth),
            "--duration-s",
            str(self.duration_s),
            "--warmup-frames",
            str(self.warmup_frames),
            "--timeout-ms",
            str(self.timeout_ms),
        ]
        if self.transition_from_exposure_us is not None:
            arguments.extend(
                [
                    "--transition-from-exposure-us",
                    str(self.transition_from_exposure_us),
                ]
            )
        if self.transition_after_ms is not None:
            arguments.extend(["--transition-after-ms", str(self.transition_after_ms)])
        if self.acquisition_mode is AcquisitionMode.SNAPSHOT:
            arguments.extend(["--mode", self.acquisition_mode.value])
        return arguments


class ControlCapability(BaseModel):
    """Writable range for one SDK control."""

    minimum: int
    maximum: int
    default: int
    writable: bool


class CameraCapabilities(BaseModel):
    """Hardware capabilities used to construct a valid benchmark matrix."""

    camera_name: str
    sdk_version: str
    width: int
    height: int
    is_usb3_host: bool
    is_usb3_camera: bool
    supported_formats: list[ImageFormat]
    bandwidth: ControlCapability
    high_speed: ControlCapability | None = None

    @model_validator(mode="after")
    def require_binding_full_frame(self) -> CameraCapabilities:
        """Reject capability data from a camera other than the binding target."""
        if self.width != FULL_FRAME_WIDTH or self.height != FULL_FRAME_HEIGHT:
            raise ValueError(
                "GRE-183 requires the ASI662MC native 1920x1080 frame size"
            )
        return self


class TimingSummary(BaseModel):
    """Exact timing distribution summary in milliseconds."""

    count: int = Field(ge=0)
    minimum_ms: float | None
    mean_ms: float | None
    p50_ms: float | None
    p95_ms: float | None
    p99_ms: float | None
    maximum_ms: float | None


class TransitionMetrics(BaseModel):
    """Live exposure-change interruption measured by the native reference."""

    from_exposure_us: int = Field(ge=1)
    to_exposure_us: int = Field(ge=1)
    setting_call_ms: float = Field(ge=0)
    setting_to_first_new_frame_ms: float = Field(ge=0)
    frames_before_first_new: int = Field(ge=0)
    first_new_capture_call_ms: float = Field(ge=0)
    classification_threshold_ms: float | None = Field(default=None, gt=0)


class FrameSignalSummary(BaseModel):
    """Untimed last-frame signal data used to classify clipping and staleness."""

    minimum: int = Field(ge=0, le=255)
    maximum: int = Field(ge=0, le=255)
    mean: float = Field(ge=0, le=255)
    zero_fraction: float = Field(ge=0, le=1)
    saturated_fraction: float = Field(ge=0, le=1)


class CaptureResult(BaseModel):
    """Common result emitted by every capture runner."""

    schema_version: Literal[1] = SCHEMA_VERSION
    runner: RunnerKind
    scenario: Scenario
    sdk_version: str
    camera_name: str
    elapsed_s: float = Field(gt=0)
    frames: int = Field(ge=0)
    unique_frames: int = Field(ge=0)
    adjacent_duplicates: int = Field(ge=0)
    cadence_fps: float = Field(ge=0)
    unique_cadence_fps: float = Field(ge=0)
    sdk_dropped_start: int = Field(ge=0)
    sdk_dropped_end: int = Field(ge=0)
    sdk_dropped_delta: int = Field(ge=0)
    capture_errors: int = Field(ge=0)
    corrupt_frames: int = Field(ge=0)
    pipeline_drops: int = Field(ge=0)
    buffer_allocations: int = Field(ge=0)
    buffer_allocation_bytes: int = Field(ge=0)
    downstream_copy_bytes: int = Field(ge=0)
    crc32_last: int | None = Field(default=None, ge=0, le=0xFFFFFFFF)
    capture_call_ms: TimingSummary
    inter_frame_ms: TimingSummary
    unique_inter_frame_ms: TimingSummary
    transition: TransitionMetrics | None = None
    frame_signal: FrameSignalSummary | None = None
    notes: list[str] = Field(default_factory=list)

    @model_validator(mode="after")
    def validate_counters(self) -> CaptureResult:
        """Reject internally inconsistent runner output."""
        if self.unique_frames > self.frames:
            raise ValueError("unique_frames cannot exceed frames")
        if self.adjacent_duplicates > self.frames:
            raise ValueError("adjacent_duplicates cannot exceed frames")
        if self.sdk_dropped_delta != self.sdk_dropped_end - self.sdk_dropped_start:
            raise ValueError("sdk_dropped_delta does not match start/end counters")
        if self.buffer_allocation_bytes < self.scenario.buffer_bytes:
            raise ValueError("runner did not account for a complete frame buffer")
        if self.transition is None:
            if self.scenario.transition_from_exposure_us is not None:
                raise ValueError("transition scenario is missing transition metrics")
        elif (
            self.scenario.transition_from_exposure_us
            != self.transition.from_exposure_us
            or self.scenario.exposure_us != self.transition.to_exposure_us
        ):
            raise ValueError("transition metrics do not match the scenario")
        return self
