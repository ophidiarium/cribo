# Auth package initialization
from .manager import User, process, validate
from ...core.utils.helpers import Logger  # Cross-package relative import

# Package conflicts
connection = "auth_connection_string"
