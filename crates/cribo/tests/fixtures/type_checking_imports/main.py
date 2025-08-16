"""Test case for TYPE_CHECKING blocks that only contain imports."""

import typing
from module_a import function_a

if typing.TYPE_CHECKING:
    from module_b import TypeB


def use_function(x: "TypeB") -> str:
    """Use the function from module_a with type hint from module_b."""
    return function_a(x)


if __name__ == "__main__":
    # This would normally work with a proper TypeB instance
    print(use_function("test"))
