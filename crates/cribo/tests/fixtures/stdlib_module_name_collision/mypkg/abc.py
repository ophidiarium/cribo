"""Local 'abc' module that imports from stdlib 'abc'."""

from abc import ABC


class MyBaseClass(ABC):
    """Base class using stdlib ABC."""

    pass


def create_object():
    """Create an object for testing."""

    class TestObject(MyBaseClass):
        pass

    return TestObject()
