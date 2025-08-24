"""
Test case where entry file has metaclasses and wildcard imports.
This reproduces the PyYAML issue.
"""

# Wildcard import
from loader import *


# Metaclass defined FIRST (correct order in source)
class YAMLObjectMetaclass(type):
    """The metaclass for YAMLObject."""

    def __init__(cls, name, bases, kwds):
        super().__init__(name, bases, kwds)
        print(f"Metaclass __init__ for {name}")


# Class using the metaclass and wildcard imports
class YAMLObject(metaclass=YAMLObjectMetaclass):
    """
    An object that can be serialized to/from YAML.
    Uses wildcard-imported symbols in class body.
    """

    # These reference wildcard-imported classes
    yaml_loader = [Loader, FullLoader]

    def __repr__(self):
        return f"YAMLObject(loaders={len(self.yaml_loader)})"


# Test the classes work
print("Testing YAMLObject...")
obj = YAMLObject()
print(f"YAMLObject created: {obj}")
print(f"YAMLObject has metaclass: {type(YAMLObject).__name__}")
print("All tests passed!")
