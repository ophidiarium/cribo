from __future__ import annotations

from typing import Dict, Any, Union


def validate_input(data: InputData) -> bool:
    """Validate input data structure.

    Uses forward reference that requires future import.
    """
    if not isinstance(data, dict):
        return False

    return "key" in data and isinstance(data.get("numbers"), list)


def format_output(data: Any) -> FormattedOutput:
    """Format data for output."""
    return f"Formatted: {data}"


# Forward references that require future import
InputData = Dict[str, Any]
FormattedOutput = Union[str, Dict[str, Any]]
