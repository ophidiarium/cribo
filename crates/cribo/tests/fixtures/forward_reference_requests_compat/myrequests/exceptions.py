"""Exceptions module that uses compat types in class inheritance."""

from .compat import JSONDecodeError as CompatJSONDecodeError


class RequestException(Exception):
    """Base exception for all request errors."""

    pass


class InvalidJSONError(RequestException):
    """A JSON error occurred."""

    pass


# This class inherits from compat's JSONDecodeError
class JSONDecodeError(InvalidJSONError, CompatJSONDecodeError):
    """Couldn't decode the text into json."""

    pass
