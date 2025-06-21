# Core package initialization
from .utils import process as core_process
from .database import Connection as CoreConnection

# Package conflicts
Logger = "core_logger_string"
