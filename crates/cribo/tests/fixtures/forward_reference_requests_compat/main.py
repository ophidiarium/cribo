#!/usr/bin/env python3
"""Test that mimics requests' pattern causing forward reference."""

import myrequests

# Use the package functionality
jar = myrequests.CookieJar()
jar.set("test", "value")
print(f"Cookie: {jar.get('test')}")

# Test that JSONDecodeError class exists
print(
    f"JSONDecodeError MRO: {[c.__name__ for c in myrequests.JSONDecodeError.__mro__]}"
)

print("Test completed")
