# Compat module that provides compatibility classes

# Simulate the JSONDecodeError from json module
try:
    from json import JSONDecodeError
except ImportError:
    # Fallback for older Python versions
    class JSONDecodeError(ValueError):
        def __init__(self, msg, doc, pos):
            super().__init__(msg)
            self.msg = msg
            self.doc = doc
            self.pos = pos


# Also provide a bytes type reference
bytes = bytes  # This is what requests.compat does
basestring = str  # Python 3 doesn't have basestring
