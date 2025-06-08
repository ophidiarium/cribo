"""Configuration module."""

import os

class Config:
    """Configuration class."""
    
    def __init__(self):
        self.DEBUG = os.environ.get("DEBUG", "false").lower() == "true"
        self.LOG_LEVEL = os.environ.get("LOG_LEVEL", "INFO")

# Global config instance
config = Config()