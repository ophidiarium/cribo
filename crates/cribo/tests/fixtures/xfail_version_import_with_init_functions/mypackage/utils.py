"""Utilities module that needs version from sibling module."""

from .__version__ import __version__


def get_user_agent():
    """Return user agent string with version."""
    return f"mypackage/{__version__}"
