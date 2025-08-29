#!/usr/bin/env python
"""Test fixture for version imports with complex module initialization."""

import mypackage
import mypackage.utils

# Force the module to be fully initialized
print(f"User agent: {mypackage.utils.get_user_agent()}")
print(f"Version from package: {mypackage.__version__}")
