"""Utils subpackage with re-exports."""

# Re-export from helper module - this import should be preserved
from .helper import helper_function

# This unused import should also be preserved in __init__.py files
from .constants import MAX_ITEMS, DEFAULT_VALUE

__all__ = ["helper_function", "MAX_ITEMS", "DEFAULT_VALUE"]