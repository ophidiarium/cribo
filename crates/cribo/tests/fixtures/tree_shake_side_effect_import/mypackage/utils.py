# Module with side effects that imports from exceptions
# The tree-shaker bug causes CustomError to be incorrectly removed

from .exceptions import CustomError

# CRITICAL: This side effect causes the module to be excluded from tree-shaking
# But its imports are not properly tracked, leading to the bug
import os

DEFAULT_PATH = os.environ.get("DEFAULT_PATH", "/tmp")  # Side effect!


def process_data(data):
    """Process data, raising CustomError if data is None."""
    if data is None:
        raise CustomError("Data cannot be None")
    return f"Processed: {data}"


# Module-level re-export (similar to requests.utils pattern)
# This will generate: module.CustomError = CustomError
# But CustomError class definition gets tree-shaken away
__all__ = ["process_data", "CustomError", "DEFAULT_PATH"]
