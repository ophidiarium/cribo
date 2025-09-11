"""Helper functions module."""


def format_string(s: str) -> str:
    """Format a string."""
    return f"Formatted: {s}"


def unused_helper1(x: int) -> int:
    """Helper that shouldn't be initialized in process_data."""
    print("unused_helper1 was initialized - this is dead code!")
    return x * 2


def unused_helper2(y: float) -> float:
    """Another helper that shouldn't be initialized in process_data."""
    print("unused_helper2 was initialized - this is dead code!")
    return y * 3.14
