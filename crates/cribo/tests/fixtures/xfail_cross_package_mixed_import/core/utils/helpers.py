"""Helper utilities for the core package."""


def validate(value: str) -> bool:
    """Validate a string value.

    This function is imported by core.database.connection using
    a relative import (..utils.helpers), demonstrating relative
    imports within the same package hierarchy.
    """
    return bool(value and not value.startswith("_"))
