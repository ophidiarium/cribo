"""Models package with conditional imports and circular dependency handling."""

import sys

# Package version
_VERSION = "1.0.0"


def get_model_version():
    """Get the models package version."""
    return _VERSION


# Conditional type alias based on Python version
if sys.version_info >= (3, 9):
    from typing import TypeAlias

    ModelID: TypeAlias = str
else:
    ModelID = str

# Lazy import holder for BaseModel
_base_model = None


def get_base_model():
    """Lazy import of BaseModel to avoid circular imports."""
    global _base_model
    if _base_model is None:
        from .base import BaseModel

        _base_model = BaseModel
    return _base_model


# Re-export from user module
from .user import process_user

# Import-time configuration
DEFAULT_MODEL_CONFIG = {
    "version": _VERSION,
    "features": ["user_processing", "lazy_loading"],
}

# Try to import advanced model (may not exist)
try:
    from models.advanced import AdvancedModel

    HAS_ADVANCED = True
    DEFAULT_MODEL_CONFIG["features"].append("advanced_model")
except ImportError:
    HAS_ADVANCED = False
    AdvancedModel = None

# Export list
__all__ = [
    "get_model_version",
    "process_user",
    "get_base_model",
    "ModelID",
    "DEFAULT_MODEL_CONFIG",
    "HAS_ADVANCED",
]

if HAS_ADVANCED:
    __all__.append("AdvancedModel")
