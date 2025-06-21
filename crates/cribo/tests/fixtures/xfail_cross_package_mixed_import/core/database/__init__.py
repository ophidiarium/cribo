"""Database subpackage with import-time initialization and re-exports."""

# Import-time side effect: register database types
_registered_types = []


def _register_type(type_name):
    """Internal function to register database types."""
    _registered_types.append(type_name)
    return type_name


# Register some types at import time
_register_type("connection")
_register_type("cursor")

# Import from another subpackage using relative import
from ..utils.helpers import validate


# Create a database-specific validator
def validate_db_name(name: str) -> bool:
    """Validate database name with additional rules."""
    # First use the general validator
    if not validate(name):
        return False
    # Then apply database-specific rules
    return not any(char in name for char in ["/", "\\", ":"])


# Import and re-export main functionality AFTER defining validate_db_name
from .connection import connect, get_connection_info

# Import from parent package to access configuration
from .. import _initialized


# Add wrapper that checks initialization
def safe_connect(database_name: str) -> str:
    """Connect only if core is initialized."""
    if not _initialized:
        raise RuntimeError("Core package must be initialized before connecting")
    return connect(database_name)


__all__ = [
    "connect",
    "get_connection_info",
    "safe_connect",
    "validate_db_name",
    "_registered_types",
]
