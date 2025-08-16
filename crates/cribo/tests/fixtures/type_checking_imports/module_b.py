"""Module B with a type definition."""


class TypeB:
    """A type that's only used for type hints."""

    def __init__(self, value: str):
        self.value = value

    def __str__(self):
        return self.value
