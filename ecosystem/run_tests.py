#!/usr/bin/env python3
"""
Helper script to run ecosystem tests with appropriate settings for local development.

Usage:
    # Run all tests (see actual failures for xfail tests)
    python ecosystem/run_tests.py

    # Run specific test file
    python ecosystem/run_tests.py test_requests

    # Run in CI mode (xfail tests don't fail)
    python ecosystem/run_tests.py --ci
"""

import sys
import subprocess
from pathlib import Path


def main():
    args = sys.argv[1:]

    # Base pytest command
    cmd = ["python", "-m", "pytest"]

    # Check if running in CI mode
    ci_mode = "--ci" in args
    if ci_mode:
        args.remove("--ci")
        print("Running in CI mode (xfail tests will be marked as expected failures)")
    else:
        print("Running in local mode (showing actual errors for xfail tests)")
        cmd.append("--runxfail")

    # Add verbosity and better output
    cmd.extend(["-xvs", "--tb=short"])

    # Determine what to test
    if not args:
        # Run all ecosystem tests
        cmd.append("ecosystem/scenarios/test_*.py")
    else:
        # Run specific test file(s)
        for arg in args:
            if not arg.startswith("test_"):
                arg = f"test_{arg}"
            if not arg.endswith(".py"):
                arg = f"{arg}.py"
            cmd.append(f"ecosystem/scenarios/{arg}")

    print(f"Running: {' '.join(cmd)}")
    print("-" * 60)

    result = subprocess.run(cmd)
    return result.returncode


if __name__ == "__main__":
    sys.exit(main())
