"""Package that sets __module__ on all exported items, like httpx does."""

# Wildcard import brings in AsyncStream and SyncStream
from ._types import *

# Import other modules
from ._client import Client
from ._models import Request, Response

# Import main which imports client which imports back to us
from ._main import main

# Define __all__ including wildcard imported items
__all__ = [
    "AsyncStream",  # From wildcard import
    "SyncStream",  # From wildcard import
    "Client",  # From explicit import
    "Request",  # From explicit import
    "Response",  # From explicit import
    "main",  # From explicit import
]

# Set __module__ on all exported items (like httpx does)
__locals = locals()
for __name in __all__:
    if not __name.startswith("__"):
        setattr(__locals[__name], "__module__", "pkg")
