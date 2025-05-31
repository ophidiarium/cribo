"""
Command-line interface for serpen.

This module provides access to the serpen CLI when called as `python -m serpen`.
The main interface is the binary `serpen` command.
"""

import subprocess
import sys


def main() -> None:
    """Main entry point that delegates to the serpen binary."""
    try:
        # Call the serpen binary with the same arguments
        result = subprocess.run(["serpen"] + sys.argv[1:], check=False)
        sys.exit(result.returncode)
    except FileNotFoundError:
        print(
            "serpen binary not found. Please ensure serpen is properly installed.",
            file=sys.stderr,
        )
        sys.exit(1)


if __name__ == "__main__":
    main()
