"""Main package that exposes version and imports submodules."""

from .__version__ import __version__, __author__
from . import utils

__all__ = ["__version__", "__author__", "utils"]
