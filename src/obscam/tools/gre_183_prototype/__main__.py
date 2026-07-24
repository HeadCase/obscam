"""One-command orchestration for the GRE-183 evidence prototype."""

from __future__ import annotations

import argparse
from datetime import UTC, datetime
from pathlib import Path

from obscam.tools.gre_183_prototype.contract import (
    AcquisitionMode,
    ImageFormat,
    RunnerKind,
    Scenario,
)
from obscam.tools.gre_183_prototype.matrix import (
    build_mode_overlap_matrix,
    build_screening_matrix,
)
from obscam.tools.gre_183_prototype.orchestrator import (
    DEFAULT_BUILD_DIRECTORY,
    DEFAULT_SDK_INCLUDE,
    RunArtifact,
    artifact_path,
    compile_native_c,
    compile_rust_pipeline,
    execute_scenario,
    write_artifact,
    write_capabilities,
)


def _default_output_directory() -> Path:
    timestamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
    return Path("research/gre-183-native-capture/results") / timestamp


def _add_runner_argument(parser: argparse.ArgumentParser, *, multiple: bool) -> None:
    if multiple:
        parser.add_argument(
            "--runners",
            type=RunnerKind,
            nargs="+",
            default=list(RunnerKind),
        )
    else:
        parser.add_argument("--runner", type=RunnerKind, required=True)


def _add_scenario_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--format", type=ImageFormat, required=True)
    parser.add_argument("--exposure-us", type=int, required=True)
    parser.add_argument("--gain", type=int, default=250)
    parser.add_argument("--high-speed", type=int, choices=(0, 1), required=True)
    parser.add_argument("--bandwidth", type=int, required=True)
    parser.add_argument("--duration-s", type=float, required=True)
    parser.add_argument("--warmup-frames", type=int, default=5)
    parser.add_argument("--timeout-ms", type=int, required=True)
    parser.add_argument("--transition-from-exposure-us", type=int)
    parser.add_argument("--transition-after-ms", type=float)
    parser.add_argument("--mode", type=AcquisitionMode, default=AcquisitionMode.VIDEO)


def parse_args() -> argparse.Namespace:
    """Parse GRE-183 orchestration commands."""
    parser = argparse.ArgumentParser(
        description="Build and run the throwaway GRE-183 capture benchmark"
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    build_parser = subparsers.add_parser("build")
    _add_runner_argument(build_parser, multiple=True)
    build_parser.add_argument(
        "--build-directory", type=Path, default=DEFAULT_BUILD_DIRECTORY
    )
    build_parser.add_argument("--sdk-include", type=Path, default=DEFAULT_SDK_INCLUDE)

    discover_parser = subparsers.add_parser("discover")
    discover_parser.add_argument("--output", type=Path)

    run_parser = subparsers.add_parser("run")
    _add_runner_argument(run_parser, multiple=False)
    _add_scenario_arguments(run_parser)
    run_parser.add_argument(
        "--build-directory", type=Path, default=DEFAULT_BUILD_DIRECTORY
    )
    run_parser.add_argument("--sdk-include", type=Path, default=DEFAULT_SDK_INCLUDE)
    run_parser.add_argument("--output", type=Path, default=_default_output_directory())
    run_parser.add_argument("--sample-interval-s", type=float, default=1.0)

    smoke_parser = subparsers.add_parser("smoke")
    _add_runner_argument(smoke_parser, multiple=True)
    smoke_parser.add_argument("--duration-s", type=float, default=2.0)
    smoke_parser.add_argument("--gain", type=int, default=250)
    smoke_parser.add_argument(
        "--build-directory", type=Path, default=DEFAULT_BUILD_DIRECTORY
    )
    smoke_parser.add_argument("--sdk-include", type=Path, default=DEFAULT_SDK_INCLUDE)
    smoke_parser.add_argument(
        "--output", type=Path, default=_default_output_directory()
    )

    screen_parser = subparsers.add_parser("screen")
    _add_runner_argument(screen_parser, multiple=True)
    screen_parser.add_argument(
        "--exposures-ms",
        type=float,
        nargs="+",
        default=[10.0, 25.0, 50.0, 100.0, 200.0, 1000.0],
    )
    screen_parser.add_argument("--duration-s", type=float, default=60.0)
    screen_parser.add_argument("--gain", type=int, default=250)
    screen_parser.add_argument(
        "--allow-usb2",
        action="store_true",
        help="allow a diagnostic matrix that cannot establish the USB 3 ceiling",
    )
    screen_parser.add_argument(
        "--build-directory", type=Path, default=DEFAULT_BUILD_DIRECTORY
    )
    screen_parser.add_argument("--sdk-include", type=Path, default=DEFAULT_SDK_INCLUDE)
    screen_parser.add_argument(
        "--output", type=Path, default=_default_output_directory()
    )

    mode_parser = subparsers.add_parser("mode-screen")
    mode_parser.add_argument(
        "--exposures-ms",
        type=float,
        nargs="+",
        default=[10, 25, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 30000],
    )
    mode_parser.add_argument("--format", type=ImageFormat, default=ImageFormat.RAW8)
    mode_parser.add_argument("--gain", type=int, default=0)
    mode_parser.add_argument("--high-speed", type=int, choices=(0, 1), default=0)
    mode_parser.add_argument("--bandwidth", type=int, default=50)
    mode_parser.add_argument(
        "--modes",
        type=AcquisitionMode,
        nargs="+",
        default=list(AcquisitionMode),
    )
    mode_parser.add_argument(
        "--build-directory", type=Path, default=DEFAULT_BUILD_DIRECTORY
    )
    mode_parser.add_argument("--sdk-include", type=Path, default=DEFAULT_SDK_INCLUDE)
    mode_parser.add_argument("--output", type=Path, default=_default_output_directory())
    return parser.parse_args()


def _ensure_built(
    runners: list[RunnerKind], build_directory: Path, sdk_include: Path
) -> None:
    if RunnerKind.NATIVE_C in runners:
        compile_native_c(
            build_directory=build_directory,
            sdk_include=sdk_include,
        )
    if RunnerKind.RUST_PIPELINE in runners:
        compile_rust_pipeline(build_directory=build_directory)


def _scenario_from_args(args: argparse.Namespace) -> Scenario:
    return Scenario(
        image_format=args.format,
        exposure_us=args.exposure_us,
        gain=args.gain,
        high_speed=args.high_speed,
        bandwidth=args.bandwidth,
        duration_s=args.duration_s,
        warmup_frames=args.warmup_frames,
        timeout_ms=args.timeout_ms,
        transition_from_exposure_us=args.transition_from_exposure_us,
        transition_after_ms=args.transition_after_ms,
        acquisition_mode=args.mode,
    )


def _run_matrix(
    runners: list[RunnerKind],
    scenarios: list[Scenario],
    *,
    build_directory: Path,
    output_directory: Path,
    sample_interval_s: float,
) -> None:
    for scenario_index, scenario in enumerate(scenarios):
        runner_offset = scenario_index % len(runners)
        ordered_runners = runners[runner_offset:] + runners[:runner_offset]
        for runner in ordered_runners:
            output_path = artifact_path(runner, scenario, output_directory)
            if output_path.is_file():
                try:
                    existing = RunArtifact.model_validate_json(
                        output_path.read_text(encoding="utf-8")
                    )
                except ValueError:
                    pass
                else:
                    if (
                        existing.result.runner is runner
                        and existing.result.scenario == scenario
                    ):
                        print(f"already complete: {output_path}", flush=True)
                        continue
            artifact = execute_scenario(
                runner,
                scenario,
                build_directory=build_directory,
                sample_interval_s=sample_interval_s,
            )
            output_path = write_artifact(artifact, output_directory)
            print(output_path, flush=True)


def main() -> None:
    """Build or execute the requested benchmark phase."""
    args = parse_args()
    if args.command == "build":
        _ensure_built(args.runners, args.build_directory, args.sdk_include)
        return

    from obscam.tools.gre_183_prototype.python_backend import discover_capabilities

    capabilities = discover_capabilities()
    if args.command == "discover":
        if args.output is None:
            print(capabilities.model_dump_json(indent=2))
        else:
            print(write_capabilities(capabilities, args.output))
        return

    if args.command == "run":
        runners = [args.runner]
        _ensure_built(runners, args.build_directory, args.sdk_include)
        write_capabilities(capabilities, args.output)
        _run_matrix(
            runners,
            [_scenario_from_args(args)],
            build_directory=args.build_directory,
            output_directory=args.output,
            sample_interval_s=args.sample_interval_s,
        )
        return

    if args.command == "mode-screen":
        runners = [RunnerKind.NATIVE_C]
        _ensure_built(runners, args.build_directory, args.sdk_include)
        write_capabilities(capabilities, args.output)
        scenarios = build_mode_overlap_matrix(
            exposures_ms=args.exposures_ms,
            image_format=args.format,
            gain=args.gain,
            high_speed=args.high_speed,
            bandwidth=args.bandwidth,
            modes=args.modes,
        )
        _run_matrix(
            runners,
            scenarios,
            build_directory=args.build_directory,
            output_directory=args.output,
            sample_interval_s=1.0,
        )
        return

    runners = args.runners
    _ensure_built(runners, args.build_directory, args.sdk_include)
    write_capabilities(capabilities, args.output)
    if args.command == "smoke":
        scenarios = build_screening_matrix(
            capabilities,
            duration_s=args.duration_s,
            exposures_ms=[10.0],
            gain=args.gain,
        )
        baseline_bandwidth = capabilities.bandwidth.default
        baseline_high_speed = (
            capabilities.high_speed.default
            if capabilities.high_speed is not None
            else 0
        )
        scenarios = [
            scenario
            for scenario in scenarios
            if scenario.bandwidth == baseline_bandwidth
            and scenario.high_speed == baseline_high_speed
        ]
        sample_interval_s = min(0.25, args.duration_s / 2)
    else:
        if not capabilities.is_usb3_host and not args.allow_usb2:
            raise SystemExit(
                "ASI662MC is not negotiated on USB 3; reconnect it or pass "
                "--allow-usb2 for a diagnostic-only matrix"
            )
        scenarios = build_screening_matrix(
            capabilities,
            duration_s=args.duration_s,
            exposures_ms=args.exposures_ms,
            gain=args.gain,
        )
        sample_interval_s = 1.0
    _run_matrix(
        runners,
        scenarios,
        build_directory=args.build_directory,
        output_directory=args.output,
        sample_interval_s=sample_interval_s,
    )


if __name__ == "__main__":
    main()
