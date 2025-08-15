from .meta import YAMLObjectMetaclass


class YAMLObject(metaclass=YAMLObjectMetaclass):
    """An object with a metaclass"""

    yaml_tag = None

    def __str__(self):
        return f"YAMLObject(tag={self.yaml_tag})"
