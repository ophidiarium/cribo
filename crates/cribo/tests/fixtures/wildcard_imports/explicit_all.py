"""Module with no side-effects and explicit __all__."""

__all__ = ["safe_function", "SafeClass", "SAFE_CONSTANT"]


# No side-effects - just definitions
def safe_function():
    """A function that will be exported via __all__."""
    return "safe_function_result"


class SafeClass:
    """A class that will be exported via __all__."""

    def method(self):
        return "SafeClass.method_result"


SAFE_CONSTANT = "SAFE_VALUE"


# This should NOT be imported with wildcard since it's not in __all__
def _private_function():
    """This function should not be imported with wildcard."""
    return "private_result"


PRIVATE_CONSTANT = "PRIVATE_VALUE"
