"""Test how importlib.import_module registers modules in sys.modules"""

import sys
import importlib

print("=== Testing importlib sys.modules registration ===\n")

# Clear any existing entries
for key in list(sys.modules.keys()):
    if key.startswith("dynamic_"):
        del sys.modules[key]

print("1. Before any imports:")
print(f"   'dynamic_module' in sys.modules: {'dynamic_module' in sys.modules}")

# Use importlib to import a module
print("\n2. Using importlib.import_module('mymodule')...")
module1 = importlib.import_module("mymodule")
print(f"   Module imported: {module1}")
print(f"   'mymodule' in sys.modules: {'mymodule' in sys.modules}")
print(f"   sys.modules['mymodule'] is module1: {sys.modules['mymodule'] is module1}")

# Test dynamic module name
module_name = "package.submodule"
print(f"\n3. Using importlib.import_module('{module_name}')...")
module2 = importlib.import_module(module_name)
print(f"   Module imported: {module2}")
print(f"   '{module_name}' in sys.modules: {module_name in sys.modules}")
print(
    f"   sys.modules['{module_name}'] is module2: {sys.modules[module_name] is module2}"
)

# Test what happens with a string variable
print("\n4. Testing with dynamic string variable:")
dynamic_name = "package" + "." + "submodule"  # Constructed at runtime
print(f"   dynamic_name = '{dynamic_name}'")
module3 = importlib.import_module(dynamic_name)
print(f"   Module imported: {module3}")
print(f"   module3 is module2: {module3 is module2}")

# Test creating a module that doesn't exist as a file
print("\n5. Creating a module dynamically:")
import types

fake_module = types.ModuleType("fake_dynamic_module")
fake_module.test_var = "I'm fake!"
sys.modules["fake_dynamic_module"] = fake_module

# Now try to import it
imported_fake = importlib.import_module("fake_dynamic_module")
print(f"   imported_fake: {imported_fake}")
print(f"   imported_fake.test_var: {imported_fake.test_var}")
print(f"   imported_fake is fake_module: {imported_fake is fake_module}")

# Test __import__ vs importlib
print("\n6. Comparing __import__ vs importlib.import_module:")
# Clear first
if "mymodule" in sys.modules:
    del sys.modules["mymodule"]

# Use __import__
print("   Using __import__('mymodule')...")
module4 = __import__("mymodule")
print(f"   'mymodule' in sys.modules: {'mymodule' in sys.modules}")

# Clear again
del sys.modules["mymodule"]

# Use importlib
print("   Using importlib.import_module('mymodule')...")
module5 = importlib.import_module("mymodule")
print(f"   'mymodule' in sys.modules: {'mymodule' in sys.modules}")

print("\n=== Summary ===")
print("importlib.import_module DOES register modules in sys.modules!")
print("It behaves exactly like regular import statements.")
