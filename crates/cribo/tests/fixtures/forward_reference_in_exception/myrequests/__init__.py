"""Main package that mimics requests' pattern"""

# Import submodule like requests does
from . import compat

# Re-export some specific attributes at package level
# This is similar to how requests exposes certain compat attributes
JSONDecodeError = compat.JSONDecodeError
MutableMapping = compat.MutableMapping
