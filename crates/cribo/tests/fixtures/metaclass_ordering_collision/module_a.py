"""First module with metaclass and class - will have original names."""


class YAMLObjectMetaclass(type):
    """The metaclass for YAMLObject."""

    def __init__(cls, name, bases, kwds):
        super(YAMLObjectMetaclass, cls).__init__(name, bases, kwds)
        if "yaml_tag" in kwds and kwds["yaml_tag"] is not None:
            cls._registered_a = True


class YAMLObject(metaclass=YAMLObjectMetaclass):
    """An object with metaclass from module_a."""

    yaml_tag = None

    def __init__(self):
        pass
