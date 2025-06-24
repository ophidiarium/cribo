"""Test how Python deduplicates modules loaded from different paths"""

import sys
import os
import importlib

print("=== Testing path-based module loading ===\n")

# Add both the current directory and the package directory to sys.path
current_dir = os.path.dirname(os.path.abspath(__file__))
package_dir = os.path.join(current_dir, "package")

# Import submodule as a top-level module by adding package/ to sys.path
sys.path.insert(0, package_dir)
submodule_as_toplevel = importlib.import_module("submodule")
print(f"1. Imported 'submodule' (from package/ in sys.path)")
print(f"   Module: {submodule_as_toplevel}")
print(f"   ID: {id(submodule_as_toplevel)}")
print(f"   sys.modules key: 'submodule'")

# Now import it as package.submodule
from package import submodule as submodule_in_package

print(f"\n2. Imported 'package.submodule' (normal package import)")
print(f"   Module: {submodule_in_package}")
print(f"   ID: {id(submodule_in_package)}")
print(f"   sys.modules key: 'package.submodule'")

# Are they the same object?
print(f"\n3. Are they the same object? {submodule_as_toplevel is submodule_in_package}")
print(f"   submodule.sub_counter = {submodule_as_toplevel.sub_counter}")
print(f"   package.submodule.sub_counter = {submodule_in_package.sub_counter}")

# Modify one and check the other
submodule_as_toplevel.sub_counter = 999
print(f"\n4. After setting submodule.sub_counter = 999:")
print(f"   submodule.sub_counter = {submodule_as_toplevel.sub_counter}")
print(f"   package.submodule.sub_counter = {submodule_in_package.sub_counter}")

# Check sys.modules
print("\n5. sys.modules entries:")
if "submodule" in sys.modules:
    print(f"   'submodule': {sys.modules['submodule']}")
if "package.submodule" in sys.modules:
    print(f"   'package.submodule': {sys.modules['package.submodule']}")

# This demonstrates that Python treats them as DIFFERENT modules!
