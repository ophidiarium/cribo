#!/usr/bin/env python3
# Test forward reference to module attributes that are defined later
# This mimics the issue in requests where JSONDecodeError is accessed
# from requests.compat before the module is fully initialized

from mymodule import MyError
from mymodule.compat import JSONDecodeError as CompatJSONDecodeError


# This should work - creating a class that inherits from both
class CustomError(MyError, CompatJSONDecodeError):
    def __init__(self, *args):
        CompatJSONDecodeError.__init__(self, *args)
        MyError.__init__(self, str(self.args[0]) if self.args else "")


try:
    raise CustomError("Test error", "doc", 42)
except CustomError as e:
    print(f"Caught CustomError: {e}")
    print("SUCCESS")
