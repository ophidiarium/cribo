"""Another module with similar class names to trigger renaming."""


class MyMetaclass(type):
    """Another metaclass with the same name."""

    def __init__(cls, name, bases, kwds):
        super(MyMetaclass, cls).__init__(name, bases, kwds)
        cls.other_attr = "from other"


class MyObject(metaclass=MyMetaclass):
    """Another class with the same name."""

    def __repr__(self):
        return f"OtherMyObject(other_attr={self.other_attr})"
