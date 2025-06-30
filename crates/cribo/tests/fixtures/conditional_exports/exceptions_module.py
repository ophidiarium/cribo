"""Module that uses imports from compat_module (like requests.exceptions)."""

# Import from the compat module
from compat_module import JSONDecodeError as CompatJSONDecodeError


class BaseError(Exception):
    """Base error class."""

    pass


# This mimics what requests.exceptions.JSONDecodeError does
class JSONDecodeError(BaseError, CompatJSONDecodeError):
    """JSON decode error that inherits from both base and compat JSONDecodeError."""

    def __init__(self, *args, **kwargs):
        """Initialize with compat JSONDecodeError first."""
        CompatJSONDecodeError.__init__(self, *args)
        BaseError.__init__(self, *self.args, **kwargs)
