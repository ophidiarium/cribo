#!/usr/bin/env python3
"""
Simulates a software architecture with:
- A logger that needs configuration
- A configuration system that logs its initialization
- An application that uses both
- Utilities that are used everywhere (no circular deps)
"""

# Safe import - utils doesn't import anything from this package
from utils import format_message


# These would cause circular imports at module level, so we defer them
def main():
    print(format_message("=== Application Starting ==="))

    # Import config inside function to avoid circular dependency
    # (config imports logger at module level to log its initialization)
    from config import Config

    config = Config()
    config.ensure_logger_configured()
    print(format_message(f"Configuration loaded: debug={config.debug_mode}"))

    # Import app inside function to avoid circular dependency
    # (app imports both config and logger at module level)
    from app import Application

    app = Application()
    app.run()

    print(format_message("=== Application Finished ==="))


if __name__ == "__main__":
    main()
