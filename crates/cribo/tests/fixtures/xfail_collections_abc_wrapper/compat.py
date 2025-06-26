"""compat.py - Compatibility module with side effects."""

# This module has side effects, so it will become a wrapper function
print("Loading compat module...")

from collections.abc import MutableMapping, Mapping

# Export the imported classes
__all__ = ["MutableMapping", "Mapping"]
