#!/usr/bin/env python3
# This test reproduces the issue where a module tries to import from another
# module's attribute before that module is fully initialized

import mypkg

# Use the exception class
try:
    raise mypkg.CustomJSONError("Test error", "doc", 42)
except mypkg.CustomJSONError as e:
    print(f"Caught error: {e}")
    print("SUCCESS")
