# Import that will cause the submodule to be processed
from .submodule import create_object


# Metaclass definition
class MyMetaclass(type):
    """The metaclass."""

    def __init__(cls, name, bases, kwds):
        super(MyMetaclass, cls).__init__(name, bases, kwds)
        if hasattr(cls, "tag"):
            print(f"Metaclass initialized {name} with tag {cls.tag}")


# Class using the metaclass - forward reference to MyMetaclass
class MyObject(metaclass=MyMetaclass):
    """An object with metaclass."""

    tag = "my_object"

    def __init__(self):
        self.value = "initialized"

    def __repr__(self):
        return f"MyObject(value={self.value})"
