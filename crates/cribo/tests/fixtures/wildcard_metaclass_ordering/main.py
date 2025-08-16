"""
Test fixture for wildcard imports with metaclass ordering.

This tests that when using wildcard imports, symbols from __all__
are properly included even if not directly used in the class body.
"""

import yaml_module

# Verify that the module has all expected attributes from wildcard imports
assert hasattr(yaml_module, "YAMLObject")
assert hasattr(yaml_module, "YAMLObjectMetaclass")

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
