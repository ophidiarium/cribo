import wildcard_pkg

# Test that wildcard-imported symbols work
obj = wildcard_pkg.WildcardObject()
print(f"WildcardObject: {obj}")
print(f"Tag: {obj.yaml_tag}")

# Also test the metaclass is accessible
print(f"Metaclass: {wildcard_pkg.WildcardMetaclass}")
