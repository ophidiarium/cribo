# Similar pattern to httpx - imports from submodules then uses __all__ in a loop

from ._internal import MyClass, my_func

__all__ = [
    "MyClass",
    "my_func",
]

# This is the problematic pattern from httpx
# It uses __all__ to set __module__ attributes
__locals = locals()
for __name in __all__:
    if not __name.startswith("__"):
        setattr(__locals[__name], "__module__", "mypkg")
