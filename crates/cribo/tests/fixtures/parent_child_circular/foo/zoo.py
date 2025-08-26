"""Module zoo - the end of the chain."""

# Side effect to track module initialization order
print("Initializing foo.zoo module")


class Zoo:
    """A simple class with no dependencies."""

    def format(self, value):
        """Format a value."""
        return f"[{value}]"

    def transform(self, data):
        """Transform data."""
        return f"Transformed: {data}"
