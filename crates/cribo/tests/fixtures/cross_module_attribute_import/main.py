#!/usr/bin/env python3
"""Test cross-module attribute import with circular dependencies that require init functions."""

import mypackage

# This will trigger usage of utils module through the package
print(mypackage.get_full_info())
