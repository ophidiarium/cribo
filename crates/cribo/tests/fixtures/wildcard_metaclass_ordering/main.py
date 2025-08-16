#!/usr/bin/env python3
"""
Test for correct ordering when a module has:
1. Wildcard imports from other modules
2. A metaclass definition
3. A class using that metaclass that references wildcard-imported symbols

This reproduces the PyYAML bundling issue where classes get reordered incorrectly.
"""

import yaml_module

# Test basic functionality
print("Testing YAMLObject...")
obj = yaml_module.YAMLObject()
print(f"YAMLObject created: {obj}")

# Test that metaclass was applied
print(f"YAMLObject has metaclass: {type(yaml_module.YAMLObject).__name__}")


# Create a subclass to trigger metaclass __init__
class CustomYAML(yaml_module.YAMLObject):
    yaml_tag = "!custom"


print("CustomYAML class created successfully")
print("All tests passed!")
