"""Database subpackage with import-time initialization and re-exports."""

# Import validation function from utils
from ..utils.helpers import validate

# Package-level state
_registered_types = []


def _register_type(type_name):
    """Internal function to register database types."""
    _registered_types.append(type_name)
    return type_name


# Register types at import time
_register_type("connection")
_register_type("cursor")


def validate_db_name(name: str) -> bool:
    """Validate database name with additional rules."""
    if not validate(name):
        return False
    # Additional database-specific validation
    return not any(char in name for char in ["/", "\\", ":"])


# Re-export connection module functions
from .connection import connect, get_connection_info


# Import parent state (but not during module initialization to avoid circular import)
def safe_connect(database_name: str) -> str:
    """Connect only if core is initialized."""
    # Import at function level to avoid circular dependency
    from .. import is_initialized

    if not is_initialized():
        raise RuntimeError("Core package must be initialized before connecting")
    return connect(database_name)


__all__ = [
    "connect",
    "get_connection_info",
    "safe_connect",
    "validate_db_name",
    "_registered_types",
]
