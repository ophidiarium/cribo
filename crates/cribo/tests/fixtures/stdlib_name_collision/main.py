#!/usr/bin/env python3
"""Test case for modules with same name as stdlib modules."""

from mypackage.abc import MyClass
from mypackage.pretty import format_object

# Should import from local abc.py, not stdlib abc
result = format_object(MyClass())
print(f"Result: {result}")
