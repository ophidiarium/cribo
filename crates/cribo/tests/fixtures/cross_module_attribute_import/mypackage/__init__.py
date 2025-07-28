"""Package init that creates circular dependency."""

from .utils import get_version_info
from .core import process_data


def get_full_info():
    """Combine version and processed data."""
    return f"{get_version_info()} - {process_data()}"
