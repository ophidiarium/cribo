"""
Configuration module that logs its initialization process.
This creates a circular dependency: config -> logger -> config
"""

# Module-level import of logger to log initialization
# This is safe because logger doesn't import config at module level
from logger import get_logger
from utils import format_message


class Config:
    def __init__(self):
        # Set default values first
        self.debug_mode = True
        self.app_name = "CircularImportDemo"
        self.version = "1.0.0"
        self._logger_configured = False

        # Log that we're initializing config
        logger = get_logger()
        logger.log("Initializing configuration system")

    def ensure_logger_configured(self):
        """Configure logger after config is ready"""
        if not self._logger_configured:
            logger = get_logger()
            logger.configure()
            logger.log("Configuration system initialized", "INFO")
            self._logger_configured = True

    def get_setting(self, key):
        return getattr(self, key, None)


# Global config instance
_config = None


def get_config():
    global _config
    if _config is None:
        _config = Config()
    return _config


def get_log_level():
    """Get log level from config - used by logger module"""
    # During initial config creation, use default
    if _config is None:
        return "INFO"  # Default until config is ready
    return "DEBUG" if _config.debug_mode else "INFO"
