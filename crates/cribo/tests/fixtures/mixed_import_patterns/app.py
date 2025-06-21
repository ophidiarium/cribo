"""
Application module that uses both config and logger.
No circular dependencies here - just normal imports.
"""

# These are safe module-level imports because:
# - config's circular dependency with logger is handled internally
# - app doesn't export anything that config or logger need
from config import get_config
from logger import get_logger
from utils import format_message


class Application:
    def __init__(self):
        self.config = get_config()
        self.logger = get_logger()
        self.logger.log(
            f"Creating {self.config.app_name} v{self.config.version} application instance"
        )

    def run(self):
        """Run the application with various logging examples"""
        self.logger.log("Application.run() called", "DEBUG")

        # Simulate some work
        print(format_message("Performing application tasks..."))

        # Show that both module-level and function-level imports work
        self.demonstrate_import_patterns()

        self.logger.log("Application tasks completed", "INFO")

    def demonstrate_import_patterns(self):
        """Show how the same module can be imported different ways"""
        # We already have logger from module-level import
        self.logger.log("Using module-level logger import", "DEBUG")

        # But in main.py, config is imported at function-level to avoid circularity
        print(format_message("Mixed import patterns working correctly!"))

        # We can even import again at function level (Python caches it)
        from logger import get_logger

        local_logger = get_logger()
        local_logger.log("Function-level import gives same logger instance", "DEBUG")
