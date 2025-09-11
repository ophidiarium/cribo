"""Shared state module to avoid circular imports."""

# Global list to track registrations
_registered_plugins = []


def get_state():
    """Get a copy of the current state."""
    return _registered_plugins.copy()


def add_plugin(name):
    """Add a plugin to the registry."""
    _registered_plugins.append(name)
    return name
