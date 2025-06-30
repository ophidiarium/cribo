"""Test symbol collision with class inheritance."""

# Import from both modules to create collision
from auth_module import HTTPBasicAuth, HTTPProxyAuth, make_auth
from colliding_module import HTTPBasicAuth as TokenAuth

# Test that HTTPProxyAuth can be instantiated
proxy_auth = HTTPProxyAuth("user", "pass")
print(f"Created HTTPProxyAuth: {proxy_auth}")

# Test that it has the expected base class
print(f"HTTPProxyAuth.__bases__: {HTTPProxyAuth.__bases__}")

# Test the factory function
auth = make_auth("proxy", "user2", "pass2")
print(f"Factory created auth: {auth}")

print("✓ All tests passed")
