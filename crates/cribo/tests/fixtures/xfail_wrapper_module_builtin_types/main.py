"""Test wrapper module pattern that triggers init functions with builtin types bug.

This reproduces the exact pattern from requests where:
1. Multiple modules form circular dependencies (wrapper modules)
2. One wrapper module (compat) has self-referential builtin assignments
3. Another wrapper module (utils) accesses these as attributes
4. The bundler generates init functions for wrapper modules
"""

# Import the package - this triggers wrapper module initialization
import pkg

# Use the functionality that depends on compat.bytes
result = pkg.process_data(b"test")
print(f"Result: {result}")
print("Success!")
