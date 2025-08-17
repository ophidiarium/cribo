"""Child module that defines the actual symbols."""


class MyClass:
    """A simple class to test forward references."""

    def __init__(self):
        self.value = 42

    def __repr__(self):
        return f"MyClass(value={self.value})"


def helper_function():
    """A helper function."""
    return "helper"


# Export these symbols
__all__ = ["MyClass", "helper_function"]
