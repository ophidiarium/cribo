# Exceptions module that depends on compat module
from .compat import JSONDecodeError as CompatJSONDecodeError


class MyError(Exception):
    """Base error class"""

    pass


class MyJSONError(MyError, CompatJSONDecodeError):
    """JSON decode error that inherits from both our error and compat error"""

    def __init__(self, *args):
        CompatJSONDecodeError.__init__(self, *args)
        MyError.__init__(self, str(args[0]) if args else "")
