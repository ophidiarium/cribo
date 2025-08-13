#!/usr/bin/env python3
"""Test that triggers stdlib import collision bug when bundling packages."""

# When we bundle the package directly (not through main.py),
# the bundler generates incorrect imports like "from abc import abc"
import mypkg

# Test the functionality
result = mypkg.console.test_function()
print(f"Result: {result}")

# Use the abc module
obj = mypkg.abc.create_object()
print(f"Object type: {type(obj).__name__}")

print("Test completed successfully")
