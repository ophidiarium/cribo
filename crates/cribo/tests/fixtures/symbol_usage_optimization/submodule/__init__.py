"""Submodule that re-exports from helpers."""

from .helpers import format_string, unused_helper1, unused_helper2


def process_data(data: str) -> str:
    """Process data using only format_string, not the other helpers."""
    # Only format_string is actually used in the function body
    # unused_helper1 and unused_helper2 should NOT be initialized
    return format_string(data)


# Re-export for external use
__all__ = ["process_data", "format_string", "unused_helper1", "unused_helper2"]
