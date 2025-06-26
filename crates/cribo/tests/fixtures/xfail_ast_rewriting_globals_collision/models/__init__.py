# Models package init
from .user import User, process_user
from .base import initialize

# Package-level name conflicts
Logger = lambda x: f"models_logger_{x}"
process = "models_process"
