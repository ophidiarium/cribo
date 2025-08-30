"""Models module that also imports from parent."""

# Also import streams from parent package
from . import AsyncStream, SyncStream


class Request:
    """Request model."""

    def __init__(self):
        self.stream = SyncStream()


class Response:
    """Response model."""

    def __init__(self):
        self.stream = AsyncStream()
