# This mimics requests.exceptions importing from requests.compat
# The key is that we're importing a specific attribute from the compat module
from .compat import JSONDecodeError as CompatJSONDecodeError


class BaseException(Exception):
    """Base exception class"""

    pass


class CustomJSONError(BaseException, CompatJSONDecodeError):
    """Custom JSON error that inherits from compat's JSONDecodeError"""

    def __init__(self, *args):
        # Call parent constructors like requests does
        CompatJSONDecodeError.__init__(self, *args)
        BaseException.__init__(self, str(args[0]) if args else "")

    def __reduce__(self):
        # Mimic requests' pickle behavior
        return CompatJSONDecodeError.__reduce__(self)
