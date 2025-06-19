"""Configuration module with self-references."""

import os
from typing import Dict, Any

# Environment-based config
DEBUG = os.environ.get("DEBUG", "false").lower() == "true"
DEBUG = DEBUG  # Should be removed


class Settings:
    """Settings class with self-references."""

    # Class variables
    app_name = "TestApp"
    version = "1.0.0"

    # Class variable self-references (should be removed)
    app_name = app_name  # Should be removed
    version = version  # Should be removed

    def __init__(self):
        self.debug = DEBUG
        self.config = self._load_config()

        # Self-references in init
        self.debug = self.debug  # Should NOT be removed (attribute assignment)
        config_copy = self.config
        config_copy = config_copy  # Should be removed

    def _load_config(self) -> Dict[str, Any]:
        """Load configuration with self-references."""
        base_config = {"app_name": self.app_name, "version": self.version, "debug": DEBUG}

        # Self-reference of dict
        base_config = base_config  # Should be removed

        # Override with environment
        for key in base_config:
            # Self-reference in loop
            key = key  # Should be removed

            env_value = os.environ.get(key.upper())
            if env_value:
                env_value = env_value  # Should be removed
                base_config[key] = env_value

        return base_config

    def get(self, key: str, default: Any = None) -> Any:
        """Get config value with self-references."""
        # Parameter self-references
        key = key  # Should be removed
        default = default  # Should be removed

        value = self.config.get(key, default)
        value = value  # Should be removed
        return value


# Global config instance
CONFIG = Settings()
CONFIG = CONFIG  # Should be removed


# Function to get config
def get_setting(name: str) -> Any:
    """Get a setting with self-references."""
    name = name  # Should be removed

    result = CONFIG.get(name)
    result = result  # Should be removed
    return result


# Self-reference of function
get_setting = get_setting  # Should be removed

# Config dict with self-references
DEFAULT_CONFIG = {"timeout": 30, "retries": 3, "buffer_size": 1024}

DEFAULT_CONFIG = DEFAULT_CONFIG  # Should be removed

# Lambda self-reference
config_getter = lambda k: CONFIG.get(k)
config_getter = config_getter  # Should be removed
