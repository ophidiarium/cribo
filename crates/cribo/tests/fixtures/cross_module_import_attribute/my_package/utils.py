"""Utility module that imports version from another module"""

# This is the pattern that fails in requests
from .__version__ import __version__

# Add a side effect to force sys.modules wrapping
print("Loading utils module...")


def get_version():
    """Return the package version"""
    return __version__
