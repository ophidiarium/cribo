# This module also defines classes with the same names to cause collisions
# This simulates how PyYAML might have internal modules that redefine things

# Local definitions that might conflict
class MyMetaclass(type):
    """Local metaclass in submodule."""

    pass


class MyObject:
    """Local object in submodule."""

    pass


def create_object():
    """Factory function that imports from parent."""
    # Import from parent package
    from . import MyObject as RealObject

    return RealObject()
