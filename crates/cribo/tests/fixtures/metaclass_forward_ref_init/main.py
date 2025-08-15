import yaml_pkg

# Test the classes
obj = yaml_pkg.YAMLObject()
print(f"YAMLObject: {obj}")
print(f"Tag: {obj.yaml_tag}")

# Test that metaclass worked
print(f"Metaclass type: {type(yaml_pkg.YAMLObject)}")
