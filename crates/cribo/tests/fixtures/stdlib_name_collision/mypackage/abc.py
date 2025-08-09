"""Local module named 'abc' - should not be confused with stdlib abc."""


class MyClass:
    """A class defined in the local abc module."""

    def __init__(self):
        self.value = "local_abc_class"

    def __str__(self):
        return f"MyClass({self.value})"
