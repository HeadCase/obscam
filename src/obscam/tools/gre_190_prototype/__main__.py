"""One-command entry point for the GRE-190 prototype."""

from __future__ import annotations

import argparse
from pathlib import Path

import uvicorn

from obscam.tools.gre_190_prototype.app import create_app
from obscam.tools.gre_190_prototype.hardware_h264 import (
    HardwareH264Scenario,
    run_hardware_gate,
)
from obscam.tools.gre_190_prototype.preflight import inspect_capabilities


def main() -> None:
    """Run capability inspection or the generated-source delivery server."""
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("preflight")
    hardware = subparsers.add_parser("hardware-h264")
    hardware.add_argument("--output", type=Path, default=Path("/tmp/gre190.h264"))
    hardware.add_argument("--duration-s", type=int, default=30)
    serve = subparsers.add_parser("serve")
    serve.add_argument("--host", default="0.0.0.0")
    serve.add_argument("--port", type=int, default=8190)
    serve.add_argument("--fps", type=float, default=10)
    serve.add_argument("--quality", type=int, choices=range(1, 96), default=80)
    args = parser.parse_args()
    if args.command == "preflight":
        print(inspect_capabilities().to_json())
        return
    if args.command == "hardware-h264":
        scenario = HardwareH264Scenario(duration_s=args.duration_s)
        print(run_hardware_gate(args.output, scenario).to_json())
        return
    uvicorn.run(
        create_app(fps=args.fps, quality=args.quality),
        host=args.host,
        port=args.port,
    )


if __name__ == "__main__":
    main()
