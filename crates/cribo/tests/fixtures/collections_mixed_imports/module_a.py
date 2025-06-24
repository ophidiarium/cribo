"""Module A with side effects that uses collections."""

from collections import OrderedDict

# Side effect - module initialization
print("Module A initializing...")


def create_ordered_dict():
    """Create an OrderedDict."""
    od = OrderedDict()
    od["x"] = 1
    od["y"] = 2
    od["z"] = 3
    return od
