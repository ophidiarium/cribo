# Database package initialization
from .connection import Connection, process
from ..utils.helpers import Logger  # Relative import from parent

# Package conflicts
User = "database_user_type"  # String conflict with User classes elsewhere
