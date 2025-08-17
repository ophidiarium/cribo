"""Parent module that re-exports child module symbols."""

# First, import some other modules that define base classes
from .base import BaseClass
from .another import AnotherClass

# Then import from child module which uses the above - this creates the forward reference issue
from .child import MyClass, helper_function

# Also do a wildcard import from a subpackage init
from .subpkg import *

# Re-export in __all__ - this is what causes the parent init wrapper
# to reference MyClass before child module is initialized
__all__ = [
    "BaseClass",
    "AnotherClass",
    "MyClass",
    "helper_function",
    "SubpkgClass",
]
