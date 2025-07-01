# Another module that also imports from compat
# This creates a situation where compat needs to be initialized before multiple modules
from .compat import MutableMapping


def create_mapping():
    """Create a mutable mapping instance"""
    return MutableMapping()
