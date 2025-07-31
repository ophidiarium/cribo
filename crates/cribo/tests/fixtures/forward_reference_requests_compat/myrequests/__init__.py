"""Main package that imports from multiple submodules."""

# Import various modules (mimicking requests structure)
from . import compat
from . import cookies
from . import exceptions
from . import utils

# Import specific items from submodules (like requests does)
from .cookies import CookieJar
from .exceptions import JSONDecodeError
from .utils import decode_json

__all__ = [
    "CookieJar",
    "JSONDecodeError",
    "decode_json",
    "compat",
    "cookies",
    "exceptions",
    "utils",
]
