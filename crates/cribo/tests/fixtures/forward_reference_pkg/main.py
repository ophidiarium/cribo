#!/usr/bin/env python3
"""Test that should fail with forward reference __cribo_init issue."""

import pkg

# This should trigger the error when bundled
print(f"Value: {pkg.get_value()}")
print("Test completed")
