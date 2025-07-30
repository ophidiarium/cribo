"""Test case for forward reference to init function"""

import myrequests

# Test basic imports through the package
print(f"MutableMapping: {myrequests.MutableMapping}")
print(f"JSONDecodeError: {myrequests.JSONDecodeError}")

# Test accessing through compat submodule
print(f"compat.MutableMapping: {myrequests.compat.MutableMapping}")
print(f"compat.JSONDecodeError: {myrequests.compat.JSONDecodeError}")

# Test that we can use the imported symbols
try:
    d = myrequests.MutableMapping()
except Exception as e:
    print(f"Expected error creating abstract MutableMapping: {type(e).__name__}")

# Test that compat has other attributes
print(f"compat.builtin_str exists: {hasattr(myrequests.compat, 'builtin_str')}")
print(f"compat.is_str exists: {hasattr(myrequests.compat, 'is_str')}")

# Actually use builtin_str to prevent tree-shaking from removing it
if hasattr(myrequests.compat, "builtin_str"):
    print(f"compat.builtin_str is str: {myrequests.compat.builtin_str is str}")

# Test calling the function if it exists
if hasattr(myrequests.compat, "is_str"):
    print(f"is_str('test'): {myrequests.compat.is_str('test')}")
    print(f"is_str(123): {myrequests.compat.is_str(123)}")
