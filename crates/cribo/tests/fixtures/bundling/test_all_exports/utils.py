"""
Utility module that demonstrates __all__ handling.
Only exports helper_function and UtilityClass, not _internal_function.
"""


def helper_function():
    """A public helper function."""
    return "helper_result"


def _internal_function():
    """An internal function that should not be exposed."""
    return "internal_result"


class UtilityClass:
    """A utility class that should be exposed."""

    def __init__(self, value):
        self.value = value

    def get_value(self):
        return self.value


class _InternalClass:
    """An internal class that should not be exposed."""

    pass


# Only export these symbols - _internal_function and _InternalClass should be hidden
__all__ = ["helper_function", "UtilityClass"]
