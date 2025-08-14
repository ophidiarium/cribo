"""Submodule that imports back from package to create circular dependency."""

# Import back from package to create circular dependency
from . import package_function

# Constants
SOME_CONSTANT = "submodule-value"


def some_function(arg):
    """Function that uses both local and package-imported functionality."""
    package_result = package_function()
    return f"some_function({arg}) -> {package_result}"
