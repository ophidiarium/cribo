"""Test fixture for circular import with function reference issue.

This reproduces the _basic_auth_str issue from requests where a function
defined in one module (auth) is imported by another module (adapters),
but due to circular dependencies, the function is not available when needed.
"""

# Entry point that triggers the circular import chain
import pkg

# Try to use the functionality
adapter = pkg.adapters.Adapter()
adapter.use_auth("user", "pass")
print("SUCCESS: Function was properly available")
