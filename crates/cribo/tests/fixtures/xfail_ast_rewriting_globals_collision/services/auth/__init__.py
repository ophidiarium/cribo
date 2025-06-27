# Auth service package
from .manager import User, process, validate

# Package conflicts
Connection = lambda: "auth_package_connection"
Logger = 42
