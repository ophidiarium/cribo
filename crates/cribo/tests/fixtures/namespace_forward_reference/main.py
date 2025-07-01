#!/usr/bin/env python3
# Test that reproduces the AttributeError on SimpleNamespace
# when accessing module attributes before initialization

from mypkg.exceptions import MyJSONError

try:
    raise MyJSONError("Test error", "doc", 42)
except MyJSONError as e:
    print(f"Caught error: {e}")
    print("SUCCESS")
