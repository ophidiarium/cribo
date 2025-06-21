# Core package initialization
from .utils.helpers import Logger as CoreLogger, process as core_process

# Package-level conflicts
result = "core_package_result"
Logger = CoreLogger  # Re-export with potential conflict
