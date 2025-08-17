#!/usr/bin/env python3
"""Test case for forward reference from parent __init__ to child module symbols."""

import parent

# Test that we can access all the re-exported symbols
print(f"BaseClass: {parent.BaseClass}")
print(f"AnotherClass: {parent.AnotherClass}")
print(f"MyClass: {parent.MyClass}")
print(f"SubpkgClass: {parent.SubpkgClass}")

# Create instances to verify they work
base = parent.BaseClass()
another = parent.AnotherClass()
my = parent.MyClass()
subpkg = parent.SubpkgClass()

print(f"Base value: {base.base_value}")
print(f"Another value: {another.another_value}")
print(f"My value: {my.value}")
print(f"Subpkg value: {subpkg.subpkg_value}")

print("SUCCESS: Forward reference from parent init handled correctly")
