"""Test how importlib.import_module handles sys.modules and deduplication"""

import sys
import importlib

print("=== Testing importlib.import_module deduplication ===\n")

# First, let's import a module normally
import mymodule

print(f"1. Normal import - mymodule id: {id(mymodule)}")
print(f"   sys.modules['mymodule'] id: {id(sys.modules['mymodule'])}")

# Now use importlib to import the same module
mymodule2 = importlib.import_module("mymodule")
print(f"\n2. importlib import - mymodule2 id: {id(mymodule2)}")
print(f"   Are they the same object? {mymodule is mymodule2}")

# Import a submodule
from package import submodule

print(f"\n3. Normal submodule import - submodule id: {id(submodule)}")
print(f"   sys.modules['package.submodule'] id: {id(sys.modules['package.submodule'])}")

# Use importlib with full module path
submodule2 = importlib.import_module("package.submodule")
print(f"\n4. importlib full path - submodule2 id: {id(submodule2)}")
print(f"   Are they the same object? {submodule is submodule2}")

# Use importlib with relative import
submodule3 = importlib.import_module(".submodule", "package")
print(f"\n5. importlib relative - submodule3 id: {id(submodule3)}")
print(f"   Are they the same object? {submodule is submodule3}")

# Show what's in sys.modules
print("\n=== sys.modules entries ===")
for key in sorted(sys.modules.keys()):
    if key.startswith(("mymodule", "package")):
        print(f"  {key}: {sys.modules[key]}")

# Test modification propagation
print("\n=== Testing modification propagation ===")
mymodule.test_value = "Modified!"
print(f"Set mymodule.test_value = 'Modified!'")
print(f"mymodule2.test_value = {mymodule2.test_value}")
print(f"sys.modules['mymodule'].test_value = {sys.modules['mymodule'].test_value}")

# Test what happens if we delete from sys.modules and reimport
print("\n=== Testing sys.modules deletion and reimport ===")
print(f"Original mymodule.counter = {mymodule.counter}")
del sys.modules["mymodule"]
mymodule_new = importlib.import_module("mymodule")
print(f"After reimport mymodule_new.counter = {mymodule_new.counter}")
print(f"Are they the same object? {mymodule is mymodule_new}")
print(f"Original mymodule still has counter = {mymodule.counter}")
