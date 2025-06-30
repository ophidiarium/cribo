"""Module with conditional imports inside if/else blocks."""

# Test case 1: Simple conditional import
has_simplejson = False
try:
    import simplejson as json

    has_simplejson = True
except ImportError:
    import json

# Test case 2: Conditional import with if/else
if has_simplejson:
    from simplejson import JSONDecodeError
else:
    from json import JSONDecodeError

# Test case 3: Simpler conditional assignment
import sys

if sys.version_info[0] >= 3:
    # Python 3
    builtin_str = str
    my_str = str
else:
    # Python 2 (won't execute in Python 3)
    builtin_str = str
    my_str = unicode

# Define basestring for Python 3
basestring = (str, bytes)

# Test case 4: Nested conditionals
if has_simplejson:
    if hasattr(json, "JSONDecodeError"):
        # Already imported above
        pass
    else:
        # Fallback
        JSONDecodeError = ValueError
else:
    # Already imported from json above
    pass


# Export a function that uses the conditionally imported items
def decode_json(text):
    """Decode JSON using the conditionally imported decoder."""
    try:
        return json.loads(text)
    except JSONDecodeError as e:
        return None
