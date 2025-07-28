#!/usr/bin/env python3
# Test case that reproduces the namespace attribute error
# where a module attribute is accessed before the namespace is initialized

from mypkg.exceptions import MyJSONError

try:
    raise MyJSONError("Test error", "doc", 42)
except MyJSONError as e:
    print(f"Caught error: {e}")
    print("SUCCESS")
