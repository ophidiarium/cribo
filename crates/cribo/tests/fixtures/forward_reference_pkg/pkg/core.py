"""Core module that uses compat at top level."""

# This import at top level might trigger the issue
from .compat import CONSTANT, BaseClass

# Top-level usage that might force early evaluation
_cached_value = f"Cached: {CONSTANT}"


class DerivedClass(BaseClass):
    """Class that inherits from compat."""

    pass


def get_value():
    """Get the cached value."""
    return _cached_value
