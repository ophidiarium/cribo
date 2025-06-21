# Package initialization with re-exports that create more conflicts
from .models.user import User as PackageUser
from .core.utils.helpers import validate as package_validate

# Add package-level constants that conflict
Logger = "package_level_logger"
process = lambda x: f"package_process: {x}"

# Re-export with aliases
__all__ = ["PackageUser", "package_validate", "Logger", "process"]
