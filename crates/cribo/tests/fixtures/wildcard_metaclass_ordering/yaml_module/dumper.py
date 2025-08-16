# Dumper class that is wildcard-imported
__all__ = ["BaseDumper", "Dumper", "SafeDumper"]


class BaseDumper:
    """Base dumper class"""

    def __init__(self):
        self.name = "BaseDumper"


class Dumper(BaseDumper):
    """Standard YAML dumper"""

    def __init__(self):
        super().__init__()
        self.name = "Dumper"


class SafeDumper(BaseDumper):
    """Safe YAML dumper"""

    def __init__(self):
        super().__init__()
        self.name = "SafeDumper"
