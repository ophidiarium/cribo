"""Module named 'class' - a Python keyword that can't be used in import statements."""


class MetaClass:
    def __init__(self, name):
        self.name = name
        self.type = "metaclass"

    def describe(self):
        return f"This is a {self.type} named {self.name}"


def create_class_instance(class_name):
    return MetaClass(class_name)


CLASS_TYPES = ["abstract", "concrete", "meta"]
DEFAULT_CLASS_NAME = "KeywordModule"
