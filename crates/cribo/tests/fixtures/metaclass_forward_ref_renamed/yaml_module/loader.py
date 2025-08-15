# Simple loader module
__all__ = ["Loader", "Dumper"]


class Loader:
    @classmethod
    def add_constructor(cls, tag, constructor):
        print(f"Loader: Registering {tag}")


class Dumper:
    @classmethod
    def add_representer(cls, data_type, representer):
        print(f"Dumper: Registering {data_type}")
