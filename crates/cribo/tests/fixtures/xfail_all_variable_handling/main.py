# Test various __all__ handling scenarios
from simple_module import public_func, CONSTANT
from nested_package import exported_from_init
from nested_package.submodule import sub_function
from conflict_module import message

print("Testing simple module exports:")
print(f"public_func() = {public_func()}")
print(f"CONSTANT = {CONSTANT}")

print("\nTesting nested package exports:")
print(f"exported_from_init() = {exported_from_init()}")
print(f"sub_function() = {sub_function()}")

print("\nTesting conflict resolution:")
print(f"message = {message}")

# Test that __all__ is accessible where needed
import simple_module

# Check that expected symbols are in __all__
print(
    f"\n'public_func' in simple_module.__all__ = {'public_func' in simple_module.__all__}"
)
print(f"'CONSTANT' in simple_module.__all__ = {'CONSTANT' in simple_module.__all__}")

import nested_package.submodule as sub

# Check that expected symbols are in __all__
print(f"\n'sub_function' in submodule.__all__ = {'sub_function' in sub.__all__}")
# Note: SUB_CONSTANT may or may not be in __all__ depending on tree-shaking
# So we check for sub_function which is actually used
