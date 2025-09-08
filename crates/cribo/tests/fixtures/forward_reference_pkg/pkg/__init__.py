"""Package that triggers forward reference."""

# Import submodules
from . import compat
from . import core

# Import specific items that will trigger top-level extraction
from .core import get_value

__all__ = ["get_value", "compat", "core"]
