# Loader classes that are wildcard-imported
__all__ = ["BaseLoader", "Loader", "FullLoader", "UnsafeLoader", "SafeLoader"]


class BaseLoader:
    """Base loader class"""

    def __init__(self):
        self.name = "BaseLoader"


class Loader(BaseLoader):
    """Standard YAML loader"""

    def __init__(self):
        super().__init__()
        self.name = "Loader"


class FullLoader(BaseLoader):
    """Full featured YAML loader"""

    def __init__(self):
        super().__init__()
        self.name = "FullLoader"


class UnsafeLoader(BaseLoader):
    """Unsafe loader that can execute arbitrary Python"""

    def __init__(self):
        super().__init__()
        self.name = "UnsafeLoader"


class SafeLoader(BaseLoader):
    """Safe loader with limited functionality"""

    def __init__(self):
        super().__init__()
        self.name = "SafeLoader"
