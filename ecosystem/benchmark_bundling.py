#!/usr/bin/env python3
"""Benchmark script for ecosystem bundling.

This script bundles each ecosystem package and measures:
1. Bundling time (build time)
2. Bundle output size (file size)

The output is formatted as Bencher Metric Format (BMF) JSON for use with Bencher.dev.
"""

import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Dict, Any


def find_cribo_binary() -> str:
    """Find the cribo binary to use."""
    # Check environment variable first
    cribo_path = os.environ.get("CARGO_BIN_EXE_cribo")
    if cribo_path and Path(cribo_path).exists():
        return cribo_path

    # Try release build
    release_path = Path(__file__).parent.parent / "target/release/cribo"
    if release_path.exists():
        return str(release_path.absolute())

    # Fall back to cargo run
    return "cargo"


def bundle_package(
    package_name: str, entry_point: Path, output_path: Path
) -> Dict[str, Any]:
    """Bundle a package and measure metrics.

    Returns:
        Dict with 'time' (seconds) and 'size' (bytes) metrics
    """
    cribo_binary = find_cribo_binary()

    # Ensure output directory exists
    output_path.parent.mkdir(parents=True, exist_ok=True)

    # Validate entry point exists
    if not entry_point.exists():
        raise FileNotFoundError(f"Entry point not found: {entry_point}")

    # Remove existing output
    output_path.unlink(missing_ok=True)

    # Build command
    if cribo_binary == "cargo":
        cmd = [
            "cargo",
            "run",
            "--release",
            "--bin",
            "cribo",
            "--",
            "--entry",
            str(entry_point),
            "--output",
            str(output_path),
        ]
    else:
        cmd = [
            cribo_binary,
            "--entry",
            str(entry_point),
            "--output",
            str(output_path),
        ]

    # Measure bundling time
    start_time = time.perf_counter()

    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            check=True,
        )

        elapsed_time = time.perf_counter() - start_time

        # Get output file size
        if output_path.exists():
            file_size = output_path.stat().st_size
        else:
            raise FileNotFoundError(f"Bundle output not found: {output_path}")

        return {
            "time": elapsed_time,
            "size": file_size,
            "success": True,
        }

    except subprocess.CalledProcessError as e:
        elapsed_time = time.perf_counter() - start_time
        print(f"❌ Failed to bundle {package_name}", file=sys.stderr)
        print(f"   Error: {e.stderr}", file=sys.stderr)
        return {
            "time": elapsed_time,
            "size": 0,
            "success": False,
            "error": e.stderr,
        }


def main():
    """Run bundling benchmarks for all ecosystem packages."""
    # Define ecosystem packages to benchmark
    ecosystem_dir = Path(__file__).parent
    packages_dir = ecosystem_dir / "packages"
    output_dir = Path("target/tmp")

    packages = [
        {
            "name": "idna",
            "entry": packages_dir / "idna" / "idna",
            "output": output_dir / "idna" / "idna_bundled.py",
        },
        {
            "name": "requests",
            "entry": packages_dir / "requests" / "src" / "requests",
            "output": output_dir / "requests" / "requests_bundled.py",
        },
        {
            "name": "httpx",
            "entry": packages_dir / "httpx" / "httpx",
            "output": output_dir / "httpx" / "httpx_bundled.py",
        },
        {
            "name": "pyyaml",
            "entry": packages_dir / "pyyaml" / "lib" / "yaml",
            "output": output_dir / "pyyaml" / "yaml_bundled.py",
        },
        {
            "name": "rich",
            "entry": packages_dir / "rich" / "rich",
            "output": output_dir / "rich" / "rich_bundled.py",
        },
    ]

    # Collect metrics in Bencher Metric Format (BMF)
    bmf_metrics = {}

    print("🚀 Starting ecosystem bundling benchmarks...\n", file=sys.stderr)

    for pkg in packages:
        if not pkg["entry"].exists():
            print(f"⚠️  Skipping {pkg['name']}: entry point not found", file=sys.stderr)
            continue

        print(f"📦 Bundling {pkg['name']}...", file=sys.stderr)

        metrics = bundle_package(pkg["name"], pkg["entry"], pkg["output"])

        if metrics["success"]:
            print(
                f"   ✅ Time: {metrics['time']:.3f}s, Size: {metrics['size']:,} bytes",
                file=sys.stderr,
            )

            # Add to BMF output - each package is a benchmark with time and size measures
            bmf_metrics[pkg["name"]] = {
                "bundle_time": {"value": metrics["time"]},
                "bundle_size": {"value": metrics["size"]},
            }
        else:
            print(f"   ❌ Failed", file=sys.stderr)

    print("\n✨ Benchmark complete!\n", file=sys.stderr)

    # Output BMF JSON to stdout for Bencher
    print(json.dumps(bmf_metrics, indent=2))

    return 0


if __name__ == "__main__":
    sys.exit(main())
