# Test that foo/__init__.py is preferred over foo.py
import foo

# Should use the package version, not the module file
print(f"Imported foo from: {foo.source}")
print(f"foo.value = {foo.value}")
