"""Package init file that creates circular dependency to force wrapper module."""

# Import from submodule to create circular dependency
from .submodule import SOME_CONSTANT

# Package-level constant that uses imported value
PACKAGE_CONSTANT = f"package-{SOME_CONSTANT}"


def package_function():
    return f"package function using {SOME_CONSTANT}"
