"""
Module that mimics PyYAML's pattern where:
1. It uses wildcard imports
2. It defines YAMLObjectMetaclass and YAMLObject (causing collisions with other.py)
3. YAMLObject references symbols from wildcard imports in its class body
4. When bundled, the renamed class still references the original metaclass name
"""

# Wildcard imports
from .loader import *

# Regular imports to avoid wildcard-in-function issue
from .other import YAMLObject as OtherYAMLObject
from .other import YAMLObjectMetaclass as OtherYAMLObjectMetaclass

# Define our own versions (these will need to be renamed due to collision)
# IMPORTANT: YAMLObjectMetaclass is defined AFTER YAMLObject references it
# This is the forward reference that causes the issue


class YAMLObjectMetaclass(type):
    """The metaclass for YAMLObject"""

    def __init__(cls, name, bases, kwds):
        super(YAMLObjectMetaclass, cls).__init__(name, bases, kwds)
        if "yaml_tag" in kwds and kwds["yaml_tag"] is not None:
            cls.yaml_loader.add_constructor(cls.yaml_tag, cls.from_yaml)
            cls.yaml_dumper.add_representer(cls, cls.to_yaml)


class YAMLObject(metaclass=YAMLObjectMetaclass):
    """
    YAMLObject that references wildcard-imported symbols in class body.
    When bundled and renamed, this will become something like:
    class YAMLObject_1(metaclass=YAMLObjectMetaclass):
    But YAMLObjectMetaclass won't exist - only YAMLObjectMetaclass_1 will.
    """

    __slots__ = ()

    # These reference symbols from wildcard imports
    yaml_loader = Loader
    yaml_dumper = Dumper

    yaml_tag = None

    @classmethod
    def from_yaml(cls, loader, node):
        return f"Loading {cls.__name__}"

    @classmethod
    def to_yaml(cls, dumper, data):
        return f"Dumping {cls.__name__}"
