"""Types module with stream classes."""

__all__ = ["AsyncStream", "SyncStream"]


class AsyncStream:
    """Async stream implementation."""

    def read(self):
        return "async data"


class SyncStream:
    """Sync stream implementation."""

    def read(self):
        return "sync data"
