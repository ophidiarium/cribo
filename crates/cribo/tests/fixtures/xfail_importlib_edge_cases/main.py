# Test importlib with invalid Python identifiers
import importlib

# Can't use normal import syntax due to hyphen
# import my-module  # SyntaxError!

# Must use importlib
mod = importlib.import_module("my-module")
print(f"Value: {mod.value}")
print(f"Function result: {mod.get_value()}")

assert mod.value == 42
print("SUCCESS: Importlib works with invalid identifiers!")
