"""
Package initialization with re-exports.

This __init__.py demonstrates the pattern where imports are made but not directly used
within this file - they are re-exports for the package interface.
These imports should NOT be stripped as unused, even though they don't appear
to be used within this file itself.
"""

# These imports appear "unused" in this file but are actually re-exports
from .data_processor import process_data
from .formatter import format_data
from .config import config

# Also import from subpackage
from .utils import helper_function

# Package version - this IS used in this file
__version__ = "1.0.0"

# These are the public API of this package
__all__ = ["process_data", "format_data", "config", "helper_function", "__version__"]

# This variable uses an import, so this import should not be flagged as unused
DEBUG_MODE = config.DEBUG
