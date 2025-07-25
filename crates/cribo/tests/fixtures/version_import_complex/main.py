#!/usr/bin/env python
"""Test fixture for importing __version__ in a complex module structure."""

import mypackage
from mypackage.utils import get_user_agent

print(f"User agent: {get_user_agent()}")
print(f"Package version: {mypackage.__version__}")
