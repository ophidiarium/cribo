"""
Utility helper functions for the happy path test.
These functions have unique names that shouldn't conflict.
"""

from typing import List


def format_message(greeting: str, name: str) -> str:
    """Format a greeting message."""
    return f"{greeting}, {name}!"


def calculate_total(values: List[int]) -> int:
    """Calculate the sum of a list of integers."""
    return sum(values)


def get_version() -> str:
    """Get the version string."""
    return "1.0.0"


# A module-level variable
UTILS_VERSION = "1.0.0"
