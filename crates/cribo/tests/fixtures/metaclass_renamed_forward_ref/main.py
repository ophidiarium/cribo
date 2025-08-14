#!/usr/bin/env python
"""
Test fixture for metaclass forward reference with renaming issue.

This reproduces the bug where class renaming causes forward references
to metaclasses, similar to what happens with PyYAML.
"""

# Import both the metaclass and the class that uses it
from base import MyMetaclass, MyObject

# Also import another module that has its own version of these
from other import MyMetaclass as OtherMeta, MyObject as OtherObject

# Create instances to verify both work
obj1 = MyObject()
print(f"Object 1 created: {obj1}")

obj2 = OtherObject()
print(f"Object 2 created: {obj2}")

print("Test passed!")
