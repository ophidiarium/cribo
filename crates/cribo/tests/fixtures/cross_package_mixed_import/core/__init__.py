"""Core package with initialization logic and cross-package imports."""

# Package-level state that affects imports
_initialized = False
_config = {"debug": False}

# Import version from separate module to avoid circular imports
from .version import CORE_MODEL_VERSION

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


def is_initialized():
    """Check if core is initialized."""
    return _initialized


# NOTE: db_connect removed to avoid circular dependency
# Users should import directly from core.database

__all__ = [
    "initialize_core",
    "is_initialized",
    "validate",
    "get_config",
    "CORE_MODEL_VERSION",
]
