"""Logger module - has side effects (print), uses different aliased stdlib imports."""

import sys as python_sys
import logging as log_lib
from collections import OrderedDict as ODict
from json import dumps as json_stringify

# Side effect: print statement at module level
print("Initializing logger module...")

# Side effect: configure logging (no timestamp for deterministic output)
log_lib.basicConfig(level=log_lib.INFO, format="%(levelname)s - %(message)s")

# Create a logger instance (side effect)
logger = log_lib.getLogger(__name__)


def log_message(message: str) -> None:
    """Log a message using aliased logging module."""
    # Use aliased logging
    logger.info(f"[ALIASED LOG] {message}")

    # Also print to aliased sys.stdout
    python_sys.stdout.write(f"[STDOUT] {message}\n")
    python_sys.stdout.flush()


def get_logger_info() -> str:
    """Get information about the logger configuration."""
    # Use aliased OrderedDict
    info = ODict(
        [
            ("python_version", "3.12.0"),  # Fixed version for deterministic output
            ("logger_name", logger.name),
            ("log_level", log_lib.getLevelName(logger.level)),
            ("handlers", len(logger.handlers)),
        ]
    )

    # Use aliased json dumps
    return json_stringify(info, indent=2)
