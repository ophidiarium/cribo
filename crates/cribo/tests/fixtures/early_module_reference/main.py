"""Test case for early module reference before initialization.

This reproduces the issue where a module tries to reference another module
before it has been fully initialized through its init function.
"""

import mypkg

# Use the API that depends on sessions
response = mypkg.api.request("GET", "https://example.com")
print(f"Response: {response}")
