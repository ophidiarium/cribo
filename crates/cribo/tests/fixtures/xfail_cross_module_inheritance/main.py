"""Test cross-module inheritance with symbol collision.

This reproduces the exact error from requests where HTTPBasicAuth
is renamed to HTTPBasicAuth_1 in one module but another module
tries to inherit from HTTPBasicAuth.
"""

# Import both HTTPBasicAuth implementations to create collision
from auth import HTTPBasicAuth
from another_auth import HTTPBasicAuth as AnotherAuth

# Import the proxy auth that inherits from HTTPBasicAuth
from proxy import HTTPProxyAuth

# Test instantiation
proxy = HTTPProxyAuth("user", "pass")
print(f"Created proxy auth: {proxy}")

# Test the original auth
basic = HTTPBasicAuth("user2", "pass2")
print(f"Created basic auth: {basic}")

print("✓ All tests passed")
