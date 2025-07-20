# Compat module that provides JSONDecodeError
# This mimics how requests.compat works

try:
    # Try to import from a hypothetical json replacement
    from fakejson import JSONDecodeError
except ImportError:
    # Fall back to standard json
    from json import JSONDecodeError

# Also provide some other compatibility stuff that might be used
import collections.abc

MutableMapping = collections.abc.MutableMapping

__all__ = ["JSONDecodeError", "MutableMapping"]
