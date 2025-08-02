"""Internal utils that depends on compat, creating circular dependency."""

# Import from compat - this creates dependency but not immediate circular import
from .compat import builtin_str

# Define something that compat imports
HEADER_VALIDATORS = {
    "str": lambda x: isinstance(x, builtin_str),
    "bytes": lambda x: isinstance(x, bytes),
}


def to_native_string(value):
    """Convert to native string."""
    if isinstance(value, builtin_str):
        return value
    return str(value)
