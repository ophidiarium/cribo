"""Test conditional imports in if/else and try/except blocks."""

import conditional_module
import try_except_module

# Test importing from modules with conditional imports
print(
    "If/else import - JSONDecodeError:", hasattr(conditional_module, "JSONDecodeError")
)
print("If/else import - json:", hasattr(conditional_module, "json"))
print("If/else import - builtin_str:", hasattr(conditional_module, "builtin_str"))

# Test the actual JSONDecodeError class
try:
    raise conditional_module.JSONDecodeError("test", "doc", 0)
except conditional_module.JSONDecodeError as e:
    print("Successfully caught JSONDecodeError from conditional import")

# Test try/except imports
print("Try/except import - etree:", hasattr(try_except_module, "etree"))
print("Try/except import - ETREE_VERSION:", hasattr(try_except_module, "ETREE_VERSION"))

# Test accessing the imported module
if try_except_module.etree is not None:
    print("Successfully imported etree module")
else:
    print("etree module is None (not available)")

# Test that all expected attributes are accessible
expected_attrs = [
    "JSONDecodeError",
    "json",
    "builtin_str",
    "basestring",
    "has_simplejson",
]
for attr in expected_attrs:
    if hasattr(conditional_module, attr):
        print(f"✓ {attr} is accessible")
    else:
        print(f"✗ {attr} is NOT accessible")
