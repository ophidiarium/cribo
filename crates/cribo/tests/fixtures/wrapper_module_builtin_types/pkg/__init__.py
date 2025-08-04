"""Main package that creates wrapper module pattern like requests."""

# Import utils which imports from compat which imports from _internal_utils
# This creates the circular dependency pattern
from . import utils, compat, _internal_utils

# Import from submodules to create more dependencies
from .utils import process_data
from .compat import basestring

# Import from sessions which depends on multiple other modules
from .sessions import Session

# Re-export for API
__all__ = ["process_data", "Session", "utils", "compat"]
