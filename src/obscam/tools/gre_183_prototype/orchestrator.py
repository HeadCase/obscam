"""Build, execute, and monitor GRE-183 benchmark runners."""

from __future__ import annotations

import json
import os
import platform
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

from pydantic import BaseModel, Field

from obscam.tools.gre_183_prototype.contract import (
    CameraCapabilities,
    CaptureResult,
    RunnerKind,
    Scenario,
)

DEFAULT_SDK_INCLUDE = Path(
    "/home/gheadley/src/ASI_Camera_SDK/ASI_linux_mac_SDK_V1.38/include"
)
DEFAULT_SDK_LIBRARY = Path("/usr/local/lib/libASICamera2.so")
DEFAULT_BUILD_DIRECTORY = Path("/tmp/obscam-gre-183-build")
PROTOTYPE_DIRECTORY = Path(__file__).resolve().parent
NATIVE_C_SOURCE = PROTOTYPE_DIRECTORY / "native_c" / "runner.c"
RUST_MANIFEST = PROTOTYPE_DIRECTORY / "rust_pipeline" / "Cargo.toml"


class HostSnapshot(BaseModel):
    """Relevant host state captured immediately around a runner execution."""

    captured_at: datetime
    machine: str
    kernel: str
    load_average: tuple[float, float, float]
    memory_available_bytes: int | None
    temperature_c: float | None
    throttled: str | None
    cpu_frequency_khz: dict[str, int]
    usb_devices: str
    usb_tree: str
    kernel_usb_log: str
    relevant_processes: list[str]


class ResourceSample(BaseModel):
    """One low-frequency resource sample for a benchmark subprocess."""

    elapsed_s: float = Field(ge=0)
    user_cpu_s: float = Field(ge=0)
    system_cpu_s: float = Field(ge=0)
    rss_bytes: int = Field(ge=0)
    high_water_rss_bytes: int = Field(ge=0)
    temperature_c: float | None
    system_load_1m: float = Field(ge=0)


class ResourceSummary(BaseModel):
    """Resource samples and derived peaks for one runner execution."""

    samples: list[ResourceSample]
    peak_rss_bytes: int = Field(ge=0)
    peak_high_water_rss_bytes: int = Field(ge=0)
    peak_temperature_c: float | None
    observed_cpu_s: float = Field(ge=0)


class RunArtifact(BaseModel):
    """Capture result plus the environment needed to interpret it."""

    result: CaptureResult
    resources: ResourceSummary
    host_before: HostSnapshot
    host_after: HostSnapshot
    runner_attempts: int = Field(default=1, ge=1)
    recoverable_start_errors: list[str] = Field(default_factory=list)


class RunnerExecutionError(RuntimeError):
    """Raised when a native or Python benchmark subprocess fails."""


class ProtectedUsbDeviceError(RunnerExecutionError):
    """Raised when a production USB device disappears during a benchmark."""


@dataclass(frozen=True, slots=True)
class ProtectedUsbDevice:
    """USB identity that must remain enumerated throughout every cell."""

    vendor_id: str
    product_id: str
    name: str


PRODUCTION_ALLSKY_CAMERA = ProtectedUsbDevice(
    vendor_id="03c3",
    product_id="178a",
    name="ZWO ASI178MC",
)
DEFAULT_PROTECTED_USB_DEVICES = (PRODUCTION_ALLSKY_CAMERA,)


def is_recoverable_camera_error(error: str) -> bool:
    """Return whether a runner failure may be transient USB re-enumeration."""
    recoverable_markers = (
        "required camera ASI662MC not found",
        "Required camera ASI662MC could not be opened",
        "ASI error 5",
    )
    return any(marker in error for marker in recoverable_markers)


def _read_text(path: Path) -> str | None:
    try:
        return path.read_text().strip()
    except OSError:
        return None


def usb_device_present(
    device: ProtectedUsbDevice,
    *,
    sysfs_root: Path = Path("/sys/bus/usb/devices"),
) -> bool:
    """Check USB enumeration through sysfs without transacting with the device."""
    for device_directory in sysfs_root.iterdir():
        vendor_id = _read_text(device_directory / "idVendor")
        product_id = _read_text(device_directory / "idProduct")
        if vendor_id == device.vendor_id and product_id == device.product_id:
            return True
    return False


def _missing_protected_devices(
    protected_devices: tuple[ProtectedUsbDevice, ...],
    *,
    sysfs_root: Path,
) -> list[ProtectedUsbDevice]:
    return [
        device
        for device in protected_devices
        if not usb_device_present(device, sysfs_root=sysfs_root)
    ]


def _memory_available_bytes() -> int | None:
    memory_info = _read_text(Path("/proc/meminfo"))
    if memory_info is None:
        return None
    for line in memory_info.splitlines():
        if line.startswith("MemAvailable:"):
            return int(line.split()[1]) * 1024
    return None


def _temperature_c() -> float | None:
    raw_temperature = _read_text(Path("/sys/class/thermal/thermal_zone0/temp"))
    if raw_temperature is None:
        return None
    return int(raw_temperature) / 1000


def _cpu_frequencies_khz() -> dict[str, int]:
    frequencies: dict[str, int] = {}
    for frequency_path in sorted(
        Path("/sys/devices/system/cpu").glob("cpu[0-9]*/cpufreq/scaling_cur_freq")
    ):
        raw_frequency = _read_text(frequency_path)
        if raw_frequency is not None:
            frequencies[frequency_path.parts[-3]] = int(raw_frequency)
    return frequencies


def _command_output(command: list[str]) -> str:
    try:
        completed = subprocess.run(
            command,
            check=False,
            capture_output=True,
            text=True,
            timeout=5,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return f"unavailable: {exc}"
    output = completed.stdout.strip() or completed.stderr.strip()
    if completed.returncode != 0:
        return f"unavailable (exit {completed.returncode}): {output}"
    return output


def _kernel_usb_log() -> str:
    kernel_log = _command_output(["journalctl", "-k", "-n", "300", "--no-pager"])
    if kernel_log.startswith("unavailable"):
        return kernel_log
    relevant_lines = [
        line
        for line in kernel_log.splitlines()
        if any(token in line.casefold() for token in ("usb", "xhci", "asi"))
    ]
    return "\n".join(relevant_lines)


def _relevant_processes() -> list[str]:
    relevant_names = {"allsky", "rclone", "wg", "wireguard"}
    names: set[str] = set()
    for process_directory in Path("/proc").glob("[0-9]*"):
        process_name = _read_text(process_directory / "comm")
        if process_name is not None and any(
            token in process_name.casefold() for token in relevant_names
        ):
            names.add(process_name)
    return sorted(names)


def capture_host_snapshot() -> HostSnapshot:
    """Capture host, thermal, throttling, and USB context."""
    return HostSnapshot(
        captured_at=datetime.now(UTC),
        machine=platform.machine(),
        kernel=platform.release(),
        load_average=os.getloadavg(),
        memory_available_bytes=_memory_available_bytes(),
        temperature_c=_temperature_c(),
        throttled=_command_output(["vcgencmd", "get_throttled"]),
        cpu_frequency_khz=_cpu_frequencies_khz(),
        usb_devices=_command_output(["lsusb"]),
        usb_tree=_command_output(["lsusb", "-t"]),
        kernel_usb_log=_kernel_usb_log(),
        relevant_processes=_relevant_processes(),
    )


def parse_process_stat(stat_text: str, clock_ticks: int) -> tuple[float, float]:
    """Extract user and system CPU seconds from Linux ``/proc/<pid>/stat``."""
    closing_parenthesis = stat_text.rfind(")")
    if closing_parenthesis < 0:
        raise ValueError("process stat does not contain a command terminator")
    remaining_fields = stat_text[closing_parenthesis + 2 :].split()
    if len(remaining_fields) < 13:
        raise ValueError("process stat is missing CPU fields")
    user_ticks = int(remaining_fields[11])
    system_ticks = int(remaining_fields[12])
    return user_ticks / clock_ticks, system_ticks / clock_ticks


def parse_process_status(status_text: str) -> tuple[int, int]:
    """Extract current and high-water RSS bytes from Linux process status."""
    values: dict[str, int] = {}
    for line in status_text.splitlines():
        name, separator, raw_value = line.partition(":")
        if separator and name in {"VmRSS", "VmHWM"}:
            values[name] = int(raw_value.split()[0]) * 1024
    return values.get("VmRSS", 0), values.get("VmHWM", 0)


def _sample_process(pid: int, started_at: float) -> ResourceSample | None:
    stat_text = _read_text(Path(f"/proc/{pid}/stat"))
    status_text = _read_text(Path(f"/proc/{pid}/status"))
    if stat_text is None or status_text is None:
        return None
    clock_ticks = os.sysconf("SC_CLK_TCK")
    if not isinstance(clock_ticks, int):
        raise TypeError("SC_CLK_TCK did not return an integer")
    user_cpu_s, system_cpu_s = parse_process_stat(stat_text, clock_ticks)
    rss_bytes, high_water_rss_bytes = parse_process_status(status_text)
    return ResourceSample(
        elapsed_s=time.perf_counter() - started_at,
        user_cpu_s=user_cpu_s,
        system_cpu_s=system_cpu_s,
        rss_bytes=rss_bytes,
        high_water_rss_bytes=high_water_rss_bytes,
        temperature_c=_temperature_c(),
        system_load_1m=os.getloadavg()[0],
    )


def summarize_resources(samples: list[ResourceSample]) -> ResourceSummary:
    """Summarize low-frequency runner resource observations."""
    temperatures = [
        sample.temperature_c for sample in samples if sample.temperature_c is not None
    ]
    observed_cpu_s = 0.0
    if samples:
        first = samples[0]
        last = samples[-1]
        observed_cpu_s = (last.user_cpu_s + last.system_cpu_s) - (
            first.user_cpu_s + first.system_cpu_s
        )
    return ResourceSummary(
        samples=samples,
        peak_rss_bytes=max((sample.rss_bytes for sample in samples), default=0),
        peak_high_water_rss_bytes=max(
            (sample.high_water_rss_bytes for sample in samples), default=0
        ),
        peak_temperature_c=max(temperatures) if temperatures else None,
        observed_cpu_s=max(0.0, observed_cpu_s),
    )


def compile_native_c(
    *,
    build_directory: Path = DEFAULT_BUILD_DIRECTORY,
    sdk_include: Path = DEFAULT_SDK_INCLUDE,
) -> Path:
    """Compile the C reference with strict warnings."""
    header = sdk_include / "ASICamera2.h"
    if not header.is_file():
        raise FileNotFoundError(f"official SDK header not found at {header}")
    build_directory.mkdir(parents=True, exist_ok=True)
    output = build_directory / "gre-183-native-c"
    subprocess.run(
        [
            "cc",
            "-O2",
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            f"-I{sdk_include}",
            str(NATIVE_C_SOURCE),
            "-L/usr/local/lib",
            "-lASICamera2",
            "-lz",
            "-lm",
            "-o",
            str(output),
        ],
        check=True,
    )
    return output


def compile_rust_pipeline(*, build_directory: Path = DEFAULT_BUILD_DIRECTORY) -> Path:
    """Compile the Rust pipeline probe in release mode."""
    cargo = shutil.which("cargo")
    if cargo is None:
        raise FileNotFoundError("cargo is required to build the Rust pipeline probe")
    target_directory = build_directory / "cargo-target"
    subprocess.run(
        [
            cargo,
            "build",
            "--release",
            "--manifest-path",
            str(RUST_MANIFEST),
            "--target-dir",
            str(target_directory),
        ],
        check=True,
    )
    return target_directory / "release" / "gre-183-rust-pipeline"


def runner_command(
    runner: RunnerKind,
    scenario: Scenario,
    *,
    build_directory: Path = DEFAULT_BUILD_DIRECTORY,
) -> list[str]:
    """Return the command for one runner and common scenario."""
    arguments = scenario.runner_arguments()
    if runner is RunnerKind.PYTHON_ZWOASI:
        return [
            sys.executable,
            "-m",
            "obscam.tools.gre_183_prototype.python_runner",
            *arguments,
        ]
    if runner is RunnerKind.NATIVE_C:
        return [str(build_directory / "gre-183-native-c"), *arguments]
    return [
        str(build_directory / "cargo-target/release/gre-183-rust-pipeline"),
        *arguments,
    ]


def run_monitored(
    command: list[str],
    *,
    sample_interval_s: float = 1.0,
    protected_devices: tuple[ProtectedUsbDevice, ...] = DEFAULT_PROTECTED_USB_DEVICES,
    usb_sysfs_root: Path = Path("/sys/bus/usb/devices"),
) -> tuple[CaptureResult, ResourceSummary]:
    """Execute one runner while sampling CPU, RSS, and temperature."""
    missing_before_start = _missing_protected_devices(
        protected_devices,
        sysfs_root=usb_sysfs_root,
    )
    if missing_before_start:
        missing_names = ", ".join(device.name for device in missing_before_start)
        raise ProtectedUsbDeviceError(
            f"refusing to start because protected device is absent: {missing_names}"
        )
    process = subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    started_at = time.perf_counter()
    samples: list[ResourceSample] = []
    next_sample_at = started_at
    sentinel_interval_s = min(0.1, sample_interval_s)
    while process.poll() is None:
        missing_devices = _missing_protected_devices(
            protected_devices,
            sysfs_root=usb_sysfs_root,
        )
        if missing_devices:
            process.terminate()
            try:
                process.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.communicate()
            missing_names = ", ".join(device.name for device in missing_devices)
            raise ProtectedUsbDeviceError(
                f"aborted runner because protected device disappeared: {missing_names}"
            )
        current_time = time.perf_counter()
        if current_time >= next_sample_at:
            sample = _sample_process(process.pid, started_at)
            if sample is not None:
                samples.append(sample)
            next_sample_at = current_time + sample_interval_s
        time.sleep(sentinel_interval_s)
    stdout, stderr = process.communicate()
    missing_after_stop = _missing_protected_devices(
        protected_devices,
        sysfs_root=usb_sysfs_root,
    )
    if missing_after_stop:
        missing_names = ", ".join(device.name for device in missing_after_stop)
        raise ProtectedUsbDeviceError(
            f"protected device disappeared as runner exited: {missing_names}"
        )
    if process.returncode != 0:
        raise RunnerExecutionError(
            f"runner exited {process.returncode}: {stderr.strip()}"
        )
    output_lines = stdout.strip().splitlines()
    if not output_lines:
        raise RunnerExecutionError("runner emitted no JSON result")
    try:
        result = CaptureResult.model_validate_json(output_lines[-1])
    except ValueError as exc:
        raise RunnerExecutionError(f"runner emitted invalid result: {exc}") from exc
    return result, summarize_resources(samples)


def execute_scenario(
    runner: RunnerKind,
    scenario: Scenario,
    *,
    build_directory: Path = DEFAULT_BUILD_DIRECTORY,
    sample_interval_s: float = 1.0,
) -> RunArtifact:
    """Execute and contextualize one runner scenario."""
    recoverable_errors: list[str] = []
    retry_delays_s = (2.0, 5.0)
    for attempt in range(1, len(retry_delays_s) + 2):
        host_before = capture_host_snapshot()
        try:
            result, resources = run_monitored(
                runner_command(runner, scenario, build_directory=build_directory),
                sample_interval_s=sample_interval_s,
            )
        except RunnerExecutionError as exc:
            error = str(exc)
            if attempt > len(retry_delays_s) or not is_recoverable_camera_error(error):
                raise
            recoverable_errors.append(error)
            time.sleep(retry_delays_s[attempt - 1])
            continue
        host_after = capture_host_snapshot()
        return RunArtifact(
            result=result,
            resources=resources,
            host_before=host_before,
            host_after=host_after,
            runner_attempts=attempt,
            recoverable_start_errors=recoverable_errors,
        )
    raise AssertionError("bounded retry loop exited unexpectedly")


def artifact_path(
    runner: RunnerKind, scenario: Scenario, output_directory: Path
) -> Path:
    """Return the stable result path for one runner/scenario pair."""
    return output_directory / (
        f"{runner.value}-{scenario.image_format.value.lower()}-"
        f"{scenario.exposure_us}us-hs{scenario.high_speed}-"
        f"bw{scenario.bandwidth}.json"
    )


def write_artifact(artifact: RunArtifact, output_directory: Path) -> Path:
    """Write one scenario artifact atomically after the runner has stopped."""
    output_directory.mkdir(parents=True, exist_ok=True)
    scenario = artifact.result.scenario
    output_path = artifact_path(
        artifact.result.runner,
        scenario,
        output_directory,
    )
    temporary_path = output_path.with_suffix(".json.tmp")
    temporary_path.write_text(
        artifact.model_dump_json(indent=2) + "\n",
        encoding="utf-8",
    )
    temporary_path.replace(output_path)
    return output_path


def write_capabilities(
    capabilities: CameraCapabilities, output_directory: Path
) -> Path:
    """Write normalized SDK capability discovery data."""
    output_directory.mkdir(parents=True, exist_ok=True)
    output_path = output_directory / "capabilities.json"
    output_path.write_text(
        json.dumps(capabilities.model_dump(mode="json"), indent=2) + "\n",
        encoding="utf-8",
    )
    return output_path
