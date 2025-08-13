# This module has the same name as a stdlib module
from abc import ABC


class MyClass(ABC):
    """A class that uses ABC from the stdlib."""

    def get_name(self):
        return "MyClass instance"

    @classmethod
    def __subclasshook__(cls, other):
        """Check if this class supports a protocol."""
        return hasattr(other, "__special__")
