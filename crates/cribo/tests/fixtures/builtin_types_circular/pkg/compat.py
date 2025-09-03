"""Compatibility module that matches requests.compat pattern exactly."""

# Don't import at module level to avoid immediate circular import error

# These self-referential assignments are what the bundler skips
# in init functions, causing AttributeError when accessed as attributes
str = str
bytes = bytes
int = int

# Other assignments that work fine
builtin_str = str
basestring = (str, bytes)
integer_types = (int,)

# Import parent to create more circular dependencies
from . import __version__

# Import from collections
from collections import OrderedDict
