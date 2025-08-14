"""Base module with metaclass definitions."""


class MyMetaclass(type):
    """A metaclass that adds attributes."""

    def __init__(cls, name, bases, kwds):
        super(MyMetaclass, cls).__init__(name, bases, kwds)
        cls.base_attr = "from base"


class MyObject(metaclass=MyMetaclass):
    """A class using the metaclass."""

    def __repr__(self):
        return f"MyObject(base_attr={self.base_attr})"
