# Dumper module with dumper class
# This is imported via wildcard in __init__.py

__all__ = ["Dumper"]


class Dumper:
    """Dumper class for YAML output"""

    def __init__(self):
        self.name = "Dumper"

    @classmethod
    def add_representer(cls, data_type, representer):
        print(f"{cls.__name__}: Adding representer for {data_type}")
