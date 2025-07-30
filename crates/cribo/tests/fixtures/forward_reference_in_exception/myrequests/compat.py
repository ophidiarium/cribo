"""Compatibility module similar to requests.compat"""

import collections.abc
import json

# Import some standard library items
MutableMapping = collections.abc.MutableMapping
JSONDecodeError = json.JSONDecodeError
builtin_str = str


# Add some functionality that other modules might depend on
def is_str(value):
    """Check if value is a string"""
    return isinstance(value, str)
