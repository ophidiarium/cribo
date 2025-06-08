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

print(f"\nsimple_module.__all__ = {simple_module.__all__}")

import nested_package.submodule as sub

print(f"submodule.__all__ = {sub.__all__}")
