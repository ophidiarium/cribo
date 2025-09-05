"""Child module boo that imports another sibling."""

# Side effect to track module initialization order
print("Initializing foo.boo module")

# This import requires foo to be initialized first
# Creating the circular dependency chain
from .zoo import Zoo


def helper_function(x):
    """A helper function used by the parent package."""
    zoo = Zoo()
    return zoo.format(x)


def process_data(data):
    """Process data using zoo functionality."""
    zoo = Zoo()
    return zoo.transform(data)
