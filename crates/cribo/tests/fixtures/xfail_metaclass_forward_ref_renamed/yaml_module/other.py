# Another module that also defines YAMLObject and YAMLObjectMetaclass
# This will cause name collisions requiring renaming

__all__ = ["YAMLObject", "YAMLObjectMetaclass"]


class YAMLObjectMetaclass(type):
    """First version of the metaclass"""

    pass


class YAMLObject(metaclass=YAMLObjectMetaclass):
    """First version of YAMLObject"""

    source = "other"
