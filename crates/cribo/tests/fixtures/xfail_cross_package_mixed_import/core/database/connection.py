"""Database connection module demonstrating mixed import patterns.

This module combines:
1. Absolute import from a different package (models.user)
2. Relative import from within the same package (..utils.helpers)
3. Import from parent package's __init__.py
4. Import from sibling module's __init__.py
"""

# Cross-package absolute import (from models package to core package)
from models.user import process_user

# Relative import within the core package
from ..utils.helpers import validate as helper_validate

# Import from parent package's __init__.py
from .. import CORE_MODEL_VERSION

# Import from current package's __init__.py
from . import _registered_types, validate_db_name

# Additional cross-package import to demonstrate complex dependencies
from models import DEFAULT_MODEL_CONFIG, get_base_model

# Import-time computation using imported values
CONNECTION_METADATA = {
    "supported_types": _registered_types,
    "validator": helper_validate.__name__,
    "processor": process_user.__name__,
    "core_version": CORE_MODEL_VERSION,
    "model_config": DEFAULT_MODEL_CONFIG,
}


class Connection:
    """Connection class using mixed imports."""

    def __init__(self, database_name: str):
        # Validate using imported function
        if not validate_db_name(database_name):
            raise ValueError(f"Invalid database name: {database_name}")

        # Process using cross-package import
        self.name = process_user(database_name)
        self.metadata = CONNECTION_METADATA.copy()

        # Use lazy import if needed
        if database_name.startswith("model_"):
            BaseModel = get_base_model()
            self.model = BaseModel(database_name)
            self.metadata["model_info"] = self.model.get_info()

    def __str__(self):
        return f"Connection to {self.name}"


def connect(database_name: str) -> Connection:
    """Create a new database connection."""
    return Connection(database_name)


def get_connection_info() -> dict:
    """Get general connection information."""
    # Import at function level to demonstrate another pattern
    from ..utils.config import is_debug

    info = {
        "metadata": CONNECTION_METADATA,
        "debug_mode": is_debug(),
        "available_validators": [validate_db_name.__name__, helper_validate.__name__],
    }

    # Conditional import based on debug mode
    if is_debug():
        # Use get_config instead of importing _config directly
        from ..utils.config import get_config as get_full_config

        info["config"] = get_full_config()

    return info
