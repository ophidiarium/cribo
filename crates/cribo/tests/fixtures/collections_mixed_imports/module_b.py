"""Module B with side effects that uses collections.abc."""

from collections.abc import MutableMapping

# Side effect - module initialization
print("Module B initializing...")


def check_mapping(obj):
    """Check if object is a MutableMapping."""
    if isinstance(obj, MutableMapping):
        return f"Yes, it's a MutableMapping with {len(obj)} items"
    else:
        return "No, it's not a MutableMapping"
