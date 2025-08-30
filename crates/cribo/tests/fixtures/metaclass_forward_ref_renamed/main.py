#!/usr/bin/env python3
"""
Test case that reproduces the PyYAML metaclass renaming bug.

Expected behavior: The bundled code should work correctly.
Actual behavior: NameError because renamed class references original metaclass name.
"""

import yaml_module

# Test that module loads
print("Module loaded successfully")

# Test access to both versions
print(f"OtherYAMLObject source: {yaml_module.OtherYAMLObject.source}")
print(f"YAMLObject name: {yaml_module.YAMLObject.__name__}")


# Create a subclass using the metaclass
class MyYAML(yaml_module.YAMLObject):
    yaml_tag = "!my"


print("Created subclass successfully")

# Test the wildcard imported classes
print(f"Loader: {yaml_module.Loader.__name__}")
print(f"Dumper: {yaml_module.Dumper.__name__}")

print("All tests passed!")
