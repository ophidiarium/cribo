"""First module with potentially clashing names (no side-effects)."""

__all__ = ["shared_function", "SharedClass", "SHARED_CONSTANT", "unique_a_function"]


def shared_function():
    """Function with same name in multiple modules."""
    return "shared_function_from_module_a"


class SharedClass:
    """Class with same name in multiple modules."""

    def method(self):
        return "SharedClass_from_module_a"


SHARED_CONSTANT = "SHARED_FROM_A"


def unique_a_function():
    """Function unique to module A."""
    return "unique_a_result"


# Not in __all__, should not be imported
def _private_a():
    return "private_a"
