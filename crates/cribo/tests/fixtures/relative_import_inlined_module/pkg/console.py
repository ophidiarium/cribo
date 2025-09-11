"""Console module with side effects - will be wrapped."""

import sys
from . import errors

# Re-export the error classes
MyError = errors.MyError
AnotherError = errors.AnotherError

# Side effect to force wrapping
print("Console module loaded", file=sys.stderr)


def display(msg):
    """Display a message."""
    print(msg)
