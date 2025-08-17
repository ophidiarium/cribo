"""Package using wildcard imports and setattr pattern like httpx."""

from ._subpackage import *

__all__ = [
    "MyClass",
    "my_function",
]

# This pattern from httpx sets __module__ on exported items
__locals = locals()
for __name in __all__:
    if not __name.startswith("__"):
        setattr(__locals[__name], "__module__", "package")
