"""Module with side effects that imports and re-exports from sibling module."""

import sys
from . import definitions

# Re-export the error classes from definitions
CustomError = definitions.CustomError
AnotherError = definitions.AnotherError

# Side effect: print during module initialization
print("Module with side effects loaded", file=sys.stderr)


def display(msg):
    """Display a message."""
    print(msg)
