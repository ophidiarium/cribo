"""
Module that mimics requests.compat structure.
No __all__ defined, so all non-underscore symbols should be exported.
"""

import sys

# Regular module-level assignments
_ver = sys.version_info
is_py2 = _ver[0] == 2
is_py3 = _ver[0] == 3

# Conditional import in try/except (like simplejson)
has_simplejson = False
try:
    import simplejson as json

    has_simplejson = True
except ImportError:
    import json

# Conditional import in if/else block
if has_simplejson:
    from simplejson import JSONDecodeError
else:
    from json import JSONDecodeError

# In the original requests.compat, this would be accessible as an attribute
# even though there's no explicit assignment or __all__


# Character detection (result of function call)
def _resolve_char_detection():
    """Find supported character detection libraries."""
    chardet = None
    for lib in ("chardet", "charset_normalizer"):
        if chardet is None:
            try:
                chardet = __import__(lib)
            except ImportError:
                pass
    return chardet


chardet = _resolve_char_detection()

# More regular assignments
builtin_str = str
# Note: requests.compat does str = str but that causes issues in bundled code
# So we'll skip that problematic line
basestring = (str, bytes)

# Private variable (should not be exported)
_internal_cache = {}


# Function (should be exported)
def get_encoding_from_headers(headers):
    """Dummy function to test function exports."""
    return "utf-8"
