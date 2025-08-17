"""Subpackage that re-exports from modules using wildcards."""

from .module_a import *
from .module_b import *

__all__ = [
    "MyClass",
    "my_function",
]
