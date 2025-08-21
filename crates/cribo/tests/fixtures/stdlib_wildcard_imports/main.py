#!/usr/bin/env python3
"""Test fixture for stdlib wildcard imports and local shadowing."""

# Wildcard import from stdlib - should be preserved
from collections import *

# Test that wildcard-imported symbols work
counter = Counter(["a", "b", "c", "a", "b", "b"])
print(f"Counter: {counter}")

# OrderedDict from wildcard import
od = OrderedDict([("a", 1), ("b", 2)])
print(f"OrderedDict: {od}")

# Local shadowing of stdlib module name
import myhelper as json  # This shadows stdlib json

# The local 'json' should refer to myhelper, not stdlib
result = json.process_data({"key": "value"})
print(f"Processed data: {result}")

# Direct stdlib access should still work via _cribo
import json as real_json

data = real_json.dumps({"test": "data"})
print(f"JSON string: {data}")

# Another wildcard import to test preservation
from itertools import *

# Use a function from itertools wildcard import
pairs = list(combinations([1, 2, 3], 2))
print(f"Combinations: {pairs}")

# Test local variable shadowing a stdlib module name
collections = "I'm a local variable, not the module!"
print(f"Local collections: {collections}")

# But Counter from wildcard import should still work
another_counter = Counter(["x", "y", "z"])
print(f"Another counter: {another_counter}")

print("All tests passed!")
