"""Console module that imports various stdlib modules."""

# Import stdlib modules that might trigger the bug
import abc  # This should be stdlib abc
import typing
import enum
import threading
import ctypes

# Also import our local abc module
from . import abc as local_abc


def test_function():
    """Test function using imports."""
    # Use stdlib abc
    if abc.ABC:
        # Use typing
        my_list: typing.List[str] = ["test"]
        # Use enum
        TestEnum = enum.Enum("TestEnum", ["A", "B"])
        # Use our local abc
        obj = local_abc.create_object()
        return f"Success with {my_list} and {TestEnum.A}"
    return "Failed"
