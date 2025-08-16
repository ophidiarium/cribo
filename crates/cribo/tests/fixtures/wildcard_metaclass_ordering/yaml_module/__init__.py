# Module with wildcard imports and metaclass/class definitions
# The order here is critical - metaclass MUST come before the class using it

from .loader import *
from .dumper import *


# Metaclass defined FIRST (as in real PyYAML)
class YAMLObjectMetaclass(type):
    """The metaclass for YAMLObject."""

    def __init__(cls, name, bases, kwds):
        super().__init__(name, bases, kwds)
        if "yaml_tag" in kwds and kwds.get("yaml_tag") is not None:
            print(f"Metaclass: Registering {name} with tag {kwds['yaml_tag']}")
            # In real PyYAML, this would register with loaders/dumpers
            if hasattr(cls, "yaml_loader"):
                if isinstance(cls.yaml_loader, list):
                    for loader in cls.yaml_loader:
                        print(f"  - Would register with {loader.__name__}")
                else:
                    print(f"  - Would register with {cls.yaml_loader.__name__}")


# Class using the metaclass defined SECOND (correct order)
class YAMLObject(metaclass=YAMLObjectMetaclass):
    """
    An object that can be serialized to/from YAML.
    Uses wildcard-imported symbols in class body.
    """

    __slots__ = ()

    # These reference wildcard-imported classes
    yaml_loader = [Loader, FullLoader, UnsafeLoader]
    yaml_dumper = Dumper
    yaml_tag = None
    yaml_flow_style = None

    def __repr__(self):
        return f"YAMLObject(loaders={len(self.yaml_loader)}, dumper={self.yaml_dumper.__name__})"

    @classmethod
    def from_yaml(cls, loader, node):
        return f"Loading {cls.__name__} from YAML"

    @classmethod
    def to_yaml(cls, dumper, data):
        return f"Dumping {cls.__name__} to YAML"
