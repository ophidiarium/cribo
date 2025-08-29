# Exceptions module that references parent namespace attributes
# This is the pattern that causes the AttributeError

import mypkg


class MyError(Exception):
    """Base exception class"""

    pass


# This class tries to access mypkg.compat.JSONDecodeError
# which might not be initialized yet when bundled
class MyJSONError(MyError, mypkg.compat.JSONDecodeError):
    """JSON error that inherits from compat JSONDecodeError"""

    def __init__(self, *args):
        mypkg.compat.JSONDecodeError.__init__(self, *args)
        MyError.__init__(self, str(args[0]) if args else "")
