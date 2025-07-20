# Module that creates a class inheriting from another module's attribute
# The key is that in the bundled output, this will try to access
# mypkg.compat.JSONDecodeError before compat is initialized

from .compat import JSONDecodeError


class MyJSONError(JSONDecodeError, Exception):
    """Error class that inherits from JSONDecodeError"""

    def __init__(self, *args):
        # In bundled form, this might become mypkg.compat.JSONDecodeError.__init__
        JSONDecodeError.__init__(self, *args)
        Exception.__init__(self, str(args[0]) if args else "")
