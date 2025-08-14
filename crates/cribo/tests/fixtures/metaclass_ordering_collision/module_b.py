"""Second module with same-named metaclass and class - will get renamed."""


class YAMLObjectMetaclass(type):
    """Another metaclass with the same name."""

    def __init__(cls, name, bases, kwds):
        super(YAMLObjectMetaclass, cls).__init__(name, bases, kwds)
        if "yaml_tag" in kwds and kwds["yaml_tag"] is not None:
            cls._registered_b = True


class YAMLObject(metaclass=YAMLObjectMetaclass):
    """Another class with the same name but different metaclass."""

    yaml_tag = None

    def __init__(self):
        pass
