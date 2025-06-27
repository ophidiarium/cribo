"""Configuration module for testing regular import aliases."""

DEFAULT_CONFIG = {"debug": True, "timeout": 30}


def get_config():
    """Get the default configuration."""
    return DEFAULT_CONFIG.copy()
