"""Client module with circular import and other dependencies."""

# Import from parent
from . import AsyncStream, SyncStream

# Import from another submodule that also imports from parent
from ._models import Request, Response


class Client:
    """Client that uses streams."""

    def __init__(self):
        self.async_stream = AsyncStream()
        self.sync_stream = SyncStream()

    def make_request(self):
        req = Request()
        return Response()
