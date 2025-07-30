"""Compat module that provides base classes and utilities used by other modules."""

# Import json handling
try:
    import simplejson as json
except ImportError:
    import json

# Get the JSON decode error
if hasattr(json, "JSONDecodeError"):
    JSONDecodeError = json.JSONDecodeError
else:
    JSONDecodeError = ValueError

# Collections imports (used as base classes in other modules)
from collections.abc import MutableMapping, Mapping
from http import cookiejar as cookielib

# String types
builtin_str = str
basestring = (str, bytes)

# URL parsing functions
from urllib.parse import urlparse, urlunparse, urljoin

# Define module exports
__all__ = [
    "JSONDecodeError",
    "MutableMapping",
    "Mapping",
    "cookielib",
    "builtin_str",
    "basestring",
    "urlparse",
    "urlunparse",
    "urljoin",
    "json",
]
