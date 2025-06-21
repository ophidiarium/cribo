"""Core package with initialization logic and cross-package imports."""

# Package-level state that affects imports
_initialized = False
_config = {"debug": False}

# Store a fixed version for now to avoid circular import
CORE_MODEL_VERSION = "1.0.0"

# Re-export commonly used functions from submodules
# These will be available as core.validate and core.get_config
from .utils.helpers import validate
from .utils.config import get_config, set_config_reference

# Initialize the config module with our config reference
set_config_reference(_config)


def initialize_core(debug=False):
    """Initialize the core package with configuration."""
    global _initialized, _config
    _initialized = True
    _config["debug"] = debug

    # This affects how submodules behave
    if debug:
        print(f"Core initialized with version: {CORE_MODEL_VERSION}")

    return _initialized


# Make database functions available at package level
from .database import connect as db_connect

__all__ = [
    "initialize_core",
    "validate",
    "get_config",
    "db_connect",
    "CORE_MODEL_VERSION",
]
