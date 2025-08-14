"""Second module with potentially clashing names (no side-effects)."""

__all__ = ["shared_function", "SharedClass", "SHARED_CONSTANT", "unique_b_function"]


def shared_function():
    """Function with same name in multiple modules."""
    return "shared_function_from_module_b"


class SharedClass:
    """Class with same name in multiple modules."""

    def method(self):
        return "SharedClass_from_module_b"


SHARED_CONSTANT = "SHARED_FROM_B"


def unique_b_function():
    """Function unique to module B."""
    return "unique_b_result"


# Not in __all__, should not be imported
def _private_b():
    return "private_b"
