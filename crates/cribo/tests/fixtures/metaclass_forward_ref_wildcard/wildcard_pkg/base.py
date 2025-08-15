from .meta import WildcardMetaclass


class WildcardObject(metaclass=WildcardMetaclass):
    """An object with a metaclass"""

    yaml_tag = None

    def __str__(self):
        return f"WildcardObject(tag={self.yaml_tag})"


# Define __all__ to control what gets exported with *
__all__ = ["WildcardObject", "WildcardMetaclass"]
