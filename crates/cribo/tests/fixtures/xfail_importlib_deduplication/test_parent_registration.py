"""Test how importlib handles parent module registration"""

import sys
import importlib

print("=== Testing parent module registration ===\n")

# Clear relevant entries
for key in list(sys.modules.keys()):
    if key.startswith("package"):
        del sys.modules[key]

print("1. Initial state:")
print(f"   'package' in sys.modules: {'package' in sys.modules}")
print(f"   'package.submodule' in sys.modules: {'package.submodule' in sys.modules}")

# Import submodule directly
print("\n2. Import package.submodule directly:")
submodule = importlib.import_module("package.submodule")
print(f"   'package' in sys.modules: {'package' in sys.modules}")
print(f"   'package.submodule' in sys.modules: {'package.submodule' in sys.modules}")

# Check if we can access parent
if "package" in sys.modules:
    print(f"   sys.modules['package']: {sys.modules['package']}")
    print(
        f"   hasattr(sys.modules['package'], 'submodule'): {hasattr(sys.modules['package'], 'submodule')}"
    )

# Now test with a deeper hierarchy
print("\n3. Testing deeper hierarchy:")
# Clear first
for key in list(sys.modules.keys()):
    if key.startswith(("deep", "package")):
        del sys.modules[key]

# Create deep hierarchy
import os

os.makedirs("deep/nested/module", exist_ok=True)
with open("deep/__init__.py", "w") as f:
    f.write("print('deep/__init__.py executed')")
with open("deep/nested/__init__.py", "w") as f:
    f.write("print('deep/nested/__init__.py executed')")
with open("deep/nested/module/__init__.py", "w") as f:
    f.write("print('deep/nested/module/__init__.py executed')")
with open("deep/nested/module/leaf.py", "w") as f:
    f.write("print('deep/nested/module/leaf.py executed')\nvalue = 42")

# Import the deepest module
print("\n4. Importing deep.nested.module.leaf:")
leaf = importlib.import_module("deep.nested.module.leaf")
print(f"\nModules registered in sys.modules:")
for key in sorted(sys.modules.keys()):
    if key.startswith("deep"):
        print(f"   {key}: {sys.modules[key]}")

# Test attribute access
print("\n5. Testing attribute access through parent modules:")
print(f"   sys.modules['deep'].nested: {sys.modules['deep'].nested}")
print(f"   sys.modules['deep.nested'].module: {sys.modules['deep.nested'].module}")
print(
    f"   sys.modules['deep.nested.module'].leaf: {sys.modules['deep.nested.module'].leaf}"
)

# Cleanup
import shutil

shutil.rmtree("deep", ignore_errors=True)
