# This test case simulates the PyYAML issue where:
# 1. Classes are defined in __init__.py that reference each other
# 2. The bundler renames them due to collisions
# 3. But the renamed versions have forward references

from pkg import MyObject, create_object

obj = create_object()
print(f"Object created: {obj}")
print(f"Object tag: {obj.tag}")
