#!/usr/bin/env python3
"""
Test case for metaclass forward reference issue with wildcard imports.

This reproduces the PyYAML bundling issue where:
1. A class uses a metaclass that's defined later in the same module
2. The class body references symbols imported via wildcard imports
3. When bundled, the metaclass definition gets placed after its usage
"""

import yaml_pkg


# Test that the module loads correctly
print("Module loaded successfully")

# Test that YAMLObject is accessible (check name instead of repr)
print(f"YAMLObject name: {yaml_pkg.YAMLObject.__name__}")
print(f"YAMLObjectMetaclass name: {yaml_pkg.YAMLObjectMetaclass.__name__}")

# Test that wildcard-imported classes are accessible (check names)
print(f"Loader name: {yaml_pkg.Loader.__name__}")
print(f"FullLoader name: {yaml_pkg.FullLoader.__name__}")
print(f"Dumper name: {yaml_pkg.Dumper.__name__}")


# Test creating a subclass
class CustomYAMLObject(yaml_pkg.YAMLObject):
    yaml_tag = "!custom"

    def __init__(self, value):
        self.value = value


# The metaclass __init__ should run and print registration messages
obj = CustomYAMLObject("test")
print(f"Created custom object with value: {obj.value}")
print(f"Custom object class name: {obj.__class__.__name__}")

# Verify the yaml_loader and yaml_dumper attributes were set correctly
print(
    f"yaml_loader types: {[type(loader).__name__ for loader in CustomYAMLObject.yaml_loader]}"
)
print(f"yaml_dumper type: {type(CustomYAMLObject.yaml_dumper).__name__}")
