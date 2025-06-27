"""Test how importlib handles module names that normal import can't"""

import sys
import importlib
import os

print("=== Testing importlib edge cases with sys.modules ===\n")

# Create test modules with problematic names
os.makedirs("edge_cases", exist_ok=True)

# 1. Module starting with number
with open("edge_cases/123_module.py", "w") as f:
    f.write("print('123_module.py executed')\nvalue = 'numeric start'")

# 2. Module with reserved keyword name
with open("edge_cases/class.py", "w") as f:
    f.write("print('class.py executed')\nvalue = 'reserved keyword'")

# 3. Module with hyphen (dash)
with open("edge_cases/my-module.py", "w") as f:
    f.write("print('my-module.py executed')\nvalue = 'with hyphen'")

# 4. Module with spaces (!)
with open("edge_cases/my module.py", "w") as f:
    f.write("print('my module.py executed')\nvalue = 'with spaces'")

# 5. Module with special characters
with open("edge_cases/module@v2.py", "w") as f:
    f.write("print('module@v2.py executed')\nvalue = 'with special char'")

# Add edge_cases to path
sys.path.insert(0, "edge_cases")

print("1. Testing module starting with number (123_module):")
try:
    # This would be syntax error: import 123_module
    module1 = importlib.import_module("123_module")
    print(f"   Success! Module: {module1}")
    print(f"   module1.value = '{module1.value}'")
    print(f"   In sys.modules as: '123_module'")
    print(f"   sys.modules['123_module']: {sys.modules['123_module']}")
except Exception as e:
    print(f"   Failed: {e}")

print("\n2. Testing reserved keyword module (class):")
try:
    # This would be syntax error: import class
    module2 = importlib.import_module("class")
    print(f"   Success! Module: {module2}")
    print(f"   module2.value = '{module2.value}'")
    print(f"   In sys.modules as: 'class'")
    print(f"   sys.modules['class']: {sys.modules['class']}")
except Exception as e:
    print(f"   Failed: {e}")

print("\n3. Testing module with hyphen (my-module):")
try:
    # This would be syntax error: import my-module
    module3 = importlib.import_module("my-module")
    print(f"   Success! Module: {module3}")
    print(f"   module3.value = '{module3.value}'")
    print(f"   In sys.modules as: 'my-module'")
    print(f"   sys.modules['my-module']: {sys.modules['my-module']}")
except Exception as e:
    print(f"   Failed: {e}")

print("\n4. Testing module with spaces (my module):")
try:
    # This is impossible with normal import
    module4 = importlib.import_module("my module")
    print(f"   Success! Module: {module4}")
    print(f"   module4.value = '{module4.value}'")
    print(f"   In sys.modules as: 'my module'")
    print(f"   sys.modules['my module']: {sys.modules['my module']}")
except Exception as e:
    print(f"   Failed: {e}")

print("\n5. Testing module with special char (module@v2):")
try:
    # This would be syntax error: import module@v2
    module5 = importlib.import_module("module@v2")
    print(f"   Success! Module: {module5}")
    print(f"   module5.value = '{module5.value}'")
    print(f"   In sys.modules as: 'module@v2'")
    print(f"   sys.modules['module@v2']: {sys.modules['module@v2']}")
except Exception as e:
    print(f"   Failed: {e}")

print("\n=== Checking sys.modules keys ===")
print("All our edge case modules in sys.modules:")
for key in sorted(sys.modules.keys()):
    if any(
        key == name
        for name in ["123_module", "class", "my-module", "my module", "module@v2"]
    ):
        print(f"  '{key}': {sys.modules[key]}")

print("\n=== Testing attribute access ===")
if "123_module" in sys.modules:
    print("Can we access sys.modules['123_module'].value?")
    print(f"  Yes: {sys.modules['123_module'].value}")

if "class" in sys.modules:
    print("\nCan we access sys.modules['class'].value?")
    print(f"  Yes: {sys.modules['class'].value}")

# Test importing from these modules
print("\n=== Testing 'from' imports with importlib ===")
print("Using importlib.import_module to simulate 'from class import value':")
class_module = sys.modules.get("class")
if class_module:
    # This simulates: from class import value
    value_from_class = getattr(class_module, "value")
    print(f"  value_from_class = '{value_from_class}'")

# Clean up
sys.path.remove("edge_cases")
import shutil

shutil.rmtree("edge_cases", ignore_errors=True)
