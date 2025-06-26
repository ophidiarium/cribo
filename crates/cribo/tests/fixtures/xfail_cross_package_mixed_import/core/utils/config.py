"""Configuration utilities for the core package."""

# Module-level state that will be set by parent package
_module_config = None


def set_config_reference(config):
    """Set the configuration reference - called by parent package."""
    global _module_config
    _module_config = config


def get_config():
    """Get the current configuration."""
    if _module_config is None:
        return {"debug": False}
    return _module_config.copy()


def is_debug():
    """Check if debug mode is enabled."""
    if _module_config is None:
        return False
    return _module_config.get("debug", False)
