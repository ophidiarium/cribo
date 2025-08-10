"""Module that imports from local abc module."""

from .abc import MyClass


def format_object(obj: MyClass) -> str:
    """Format an object from the local abc module."""
    return f"Formatted: {obj}"
