"""Utility module that imports __version__ from a sibling module."""

from .__version__ import __version__


def get_user_agent():
    """Return a user agent string with version."""
    return f"mypackage/{__version__}"
