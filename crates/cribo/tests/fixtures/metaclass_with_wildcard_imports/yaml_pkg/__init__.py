# Mimics PyYAML's structure with wildcard imports and metaclass

from .loader import *
from .dumper import *


class YAMLObjectMetaclass(type):
    """
    The metaclass for YAMLObject.
    """

    def __init__(cls, name, bases, kwds):
        super(YAMLObjectMetaclass, cls).__init__(name, bases, kwds)
        if "yaml_tag" in kwds and kwds.get("yaml_tag") is not None:
            # Register the class with loaders/dumpers
            if isinstance(cls.yaml_loader, list):
                for loader in cls.yaml_loader:
                    print(f"Registering {name} with {loader.__name__}")
            else:
                print(f"Registering {name} with {cls.yaml_loader.__name__}")


class YAMLObject(metaclass=YAMLObjectMetaclass):
    """
    An object that uses a metaclass and references classes from wildcard imports.
    This mimics PyYAML's structure.
    """

    __slots__ = ()

    # These reference classes imported via wildcard imports
    yaml_loader = [Loader, FullLoader]  # From loader module
    yaml_dumper = Dumper  # From dumper module

    yaml_tag = None

    @classmethod
    def from_yaml(cls, loader, node):
        return f"Loading {cls.__name__} from YAML"

    @classmethod
    def to_yaml(cls, dumper, data):
        return f"Dumping {cls.__name__} to YAML"
