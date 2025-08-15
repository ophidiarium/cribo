class YAMLObjectMetaclass(type):
    """Metaclass that adds yaml_tag"""

    def __init__(cls, name, bases, kwds):
        super(YAMLObjectMetaclass, cls).__init__(name, bases, kwds)
        if not hasattr(cls, "yaml_tag") or cls.yaml_tag is None:
            cls.yaml_tag = f"!{name}"
