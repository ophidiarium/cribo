# Loader module with multiple loader classes
# These are imported via wildcard in __init__.py

__all__ = ["BaseLoader", "Loader", "FullLoader"]


class BaseLoader:
    """Base loader class"""

    def __init__(self):
        self.name = "BaseLoader"

    @classmethod
    def add_constructor(cls, tag, constructor):
        print(f"{cls.__name__}: Adding constructor for {tag}")


class Loader(BaseLoader):
    """Standard loader class"""

    def __init__(self):
        super().__init__()
        self.name = "Loader"


class FullLoader(BaseLoader):
    """Full loader with all features"""

    def __init__(self):
        super().__init__()
        self.name = "FullLoader"
