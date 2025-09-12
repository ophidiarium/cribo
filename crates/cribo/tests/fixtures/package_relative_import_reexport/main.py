"""Test case for package with relative imports and re-exports.

This tests the scenario where:
1. A package has a module with just class definitions (pkg.definitions)
2. Another module with side effects imports and re-exports those classes (pkg.module_with_sideeffects)
3. The main code accesses the re-exported classes
"""

import pkg.module_with_sideeffects

# Use the re-exported error class
try:
    raise pkg.module_with_sideeffects.CustomError("test")
except pkg.module_with_sideeffects.CustomError as e:
    print(f"Caught error: {e}")
