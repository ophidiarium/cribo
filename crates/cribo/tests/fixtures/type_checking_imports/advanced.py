"""Test various TYPE_CHECKING patterns that can result in empty blocks."""

import typing
from module_a import function_a

# Case 1: Single import in TYPE_CHECKING block
if typing.TYPE_CHECKING:
    from module_b import TypeB

# Case 2: Multiple imports in TYPE_CHECKING block
if typing.TYPE_CHECKING:
    from module_b import TypeB
    from module_c import TypeC, TypeD

# Case 3: Mixed content in TYPE_CHECKING block (should NOT become empty)
if typing.TYPE_CHECKING:
    from module_b import TypeB

    # This is a type alias
    OptionalTypeB = TypeB | None


def use_types(a: "TypeB", b: "TypeC", c: "OptionalTypeB") -> str:
    """Function using types from TYPE_CHECKING imports."""
    return function_a(str(a) + str(b) + str(c))


# Case 4: Nested TYPE_CHECKING (edge case)
if True:
    if typing.TYPE_CHECKING:
        from module_b import TypeB

    def nested_function(x: "TypeB"):
        return x


if __name__ == "__main__":
    print(use_types("a", "b", "c"))
