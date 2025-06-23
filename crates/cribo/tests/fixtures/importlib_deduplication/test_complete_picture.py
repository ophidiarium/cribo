"""Complete picture of importlib and sys.modules interaction"""

import sys
import importlib
import os
import shutil

print("=== COMPLETE PICTURE: importlib and sys.modules ===\n")

# Setup test structure
os.makedirs("test_pkg/sub-pkg", exist_ok=True)
os.makedirs("test_pkg/123_subpkg", exist_ok=True)

# Regular package
with open("test_pkg/__init__.py", "w") as f:
    f.write("print('test_pkg/__init__.py')")

# Subpackage with hyphen
with open("test_pkg/sub-pkg/__init__.py", "w") as f:
    f.write("print('test_pkg/sub-pkg/__init__.py')")

# Module in hyphenated package
with open("test_pkg/sub-pkg/class.py", "w") as f:
    f.write("print('test_pkg/sub-pkg/class.py')\nvalue = 'I am class.py in sub-pkg'")

# Subpackage starting with number
with open("test_pkg/123_subpkg/__init__.py", "w") as f:
    f.write("print('test_pkg/123_subpkg/__init__.py')")

# Module with special name in numeric package
with open("test_pkg/123_subpkg/for.py", "w") as f:
    f.write("print('test_pkg/123_subpkg/for.py')\nvalue = 'I am for.py in 123_subpkg'")

print("1. NORMAL IMPORTS vs IMPORTLIB")
print("-" * 50)

# Normal import of regular package
import test_pkg

print(f"import test_pkg → sys.modules['test_pkg']")

# Can't do: import test_pkg.sub-pkg (SyntaxError)
# But with importlib:
sub_pkg = importlib.import_module("test_pkg.sub-pkg")
print(f"importlib.import_module('test_pkg.sub-pkg') → sys.modules['test_pkg.sub-pkg']")

print("\n2. WHAT'S IN SYS.MODULES?")
print("-" * 50)
for key in sorted(sys.modules.keys()):
    if key.startswith("test_pkg"):
        print(f"  '{key}': {type(sys.modules[key]).__name__}")

print("\n3. ACCESSING PROBLEMATIC MODULES")
print("-" * 50)

# Import module with reserved name in hyphenated package
class_in_subpkg = importlib.import_module("test_pkg.sub-pkg.class")
print(f"Imported 'test_pkg.sub-pkg.class'")
print(f"  value = '{class_in_subpkg.value}'")
print(f"  In sys.modules as: 'test_pkg.sub-pkg.class'")

# Import from numeric package
for_in_numeric = importlib.import_module("test_pkg.123_subpkg.for")
print(f"\nImported 'test_pkg.123_subpkg.for'")
print(f"  value = '{for_in_numeric.value}'")
print(f"  In sys.modules as: 'test_pkg.123_subpkg.for'")

print("\n4. KEY INSIGHTS")
print("-" * 50)
print("a) sys.modules keys can be ANY string, including:")
print("   - Reserved keywords ('class', 'for', 'import', etc.)")
print("   - Starting with numbers ('123_module')")
print("   - Containing hyphens ('my-module')")
print("   - Containing spaces ('my module')")
print("   - Special characters ('module@v2')")

print("\nb) importlib.import_module() accepts any string as module name")
print("   - It maps directly to sys.modules[name]")
print("   - No Python identifier restrictions apply")

print("\nc) Once in sys.modules, access is only via:")
print("   - sys.modules['exact-name']")
print("   - importlib.import_module('exact-name')")
print("   - NOT via normal import statements if name is invalid")

print("\n5. DEDUPLICATION BEHAVIOR")
print("-" * 50)

# Clear and reimport
print("First import of 'test_pkg.sub-pkg.class':")
if "test_pkg.sub-pkg.class" in sys.modules:
    del sys.modules["test_pkg.sub-pkg.class"]

mod1 = importlib.import_module("test_pkg.sub-pkg.class")
print(f"  Module ID: {id(mod1)}")

print("\nSecond import of 'test_pkg.sub-pkg.class':")
mod2 = importlib.import_module("test_pkg.sub-pkg.class")
print(f"  Module ID: {id(mod2)}")
print(f"  Same object? {mod1 is mod2}")

print("\n6. IMPLICATIONS FOR BUNDLERS")
print("-" * 50)
print("- Must handle ANY string as module name")
print("- Cannot assume module names are valid Python identifiers")
print("- Deduplication key is the exact string in sys.modules")
print("- Same file can exist under multiple names in sys.modules")

# Cleanup
shutil.rmtree("test_pkg", ignore_errors=True)
