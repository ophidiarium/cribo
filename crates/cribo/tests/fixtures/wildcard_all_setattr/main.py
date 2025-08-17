"""Test wildcard imports with __all__ and setattr pattern."""

from package import MyClass, my_function

# Use the imported items to verify they work
obj = MyClass()
print(f"MyClass instance value: {obj.value}")
print(f"my_function result: {my_function()}")
print("Success!")
