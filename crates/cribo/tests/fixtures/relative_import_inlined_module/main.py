"""Test case for relative imports referencing inlined modules.

This tests the case where:
1. pkg.errors is inlined (simple module with just class definitions)
2. pkg.console is wrapped (has side effects)
3. pkg.console does 'from . import errors'
"""

import pkg.console

# Use the imported error class
try:
    raise pkg.console.MyError("test")
except pkg.console.MyError as e:
    print(f"Caught error: {e}")
