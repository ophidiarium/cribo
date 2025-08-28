"""Utils module that completes the circular chain."""

from .models import (
    BaseModel,
)  # Creates circular: utils -> models -> adapters -> auth -> utils


def format_credentials(username, password):
    """Format credentials for auth."""
    # Reference BaseModel to ensure the circular dependency
    if BaseModel:
        return f"{username}:{password}"
    return ""
