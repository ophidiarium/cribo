"""Utils module that imports from __version__ module and uses it at module level."""

from .__version__ import __version__

# Module-level code that uses the imported attribute
VERSION_PREFIX = f"v{__version__}"


def get_version_info():
    """Return version information using the imported __version__ variable."""
    return VERSION_PREFIX
