"""
Logger module that needs configuration to determine log level.
This creates a circular dependency: logger -> config -> logger
"""

# Safe import - utils doesn't create circular dependency
from utils import format_message, get_timestamp


class Logger:
    def __init__(self):
        self.log_level = "INFO"
        # We need config to set proper log level, but config needs logger to log its init
        # So we import it inside the method that needs it
        print(format_message("[Logger] Initializing logger system"))

    def configure(self):
        """Configure logger with settings from config module"""
        # Import config here to avoid circular dependency at module level
        from config import get_log_level

        self.log_level = get_log_level()
        self.log(f"Logger configured with level: {self.log_level}")

    def log(self, message, level="INFO"):
        if self._should_log(level):
            print(f"[{level}] {message}")

    def _should_log(self, level):
        levels = {"DEBUG": 0, "INFO": 1, "WARNING": 2, "ERROR": 3}
        return levels.get(level, 1) >= levels.get(self.log_level, 1)


# Global logger instance
_logger = None


def get_logger():
    global _logger
    if _logger is None:
        _logger = Logger()
    return _logger
