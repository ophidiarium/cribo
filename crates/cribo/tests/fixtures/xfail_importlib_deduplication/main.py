"""Test how importlib.import_module handles sys.modules and deduplication"""

import sys
import importlib

print("=== Testing importlib.import_module deduplication ===\n")

# First, let's import a module normally
import mymodule

# Now use importlib to import the same module
mymodule2 = importlib.import_module("mymodule")
print(f"   Are they the same object? {mymodule is mymodule2}")

# Import a submodule
from package import submodule


# Use importlib with full module path
submodule2 = importlib.import_module("package.submodule")
print(f"   Are they the same object? {submodule is submodule2}")

# Use importlib with relative import
submodule3 = importlib.import_module(".submodule", "package")
print(f"   Are they the same object? {submodule is submodule3}")

# Test modification propagation
print("\n=== Testing modification propagation ===")
mymodule.test_value = "Modified!"
print(f"Set mymodule.test_value = 'Modified!'")
print(f"mymodule2.test_value = {mymodule2.test_value}")

# Test what happens if we delete from sys.modules and reimport
mymodule_new = importlib.import_module("mymodule")
print(f"After reimport mymodule_new.counter = {mymodule_new.counter}")
print(f"Are they the same object? {mymodule is mymodule_new}")
print(f"Original mymodule still has counter = {mymodule.counter}")
