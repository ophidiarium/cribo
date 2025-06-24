"""Helper module using collections.abc."""

from collections.abc import MutableMapping, Mapping


def process_mapping(data):
    """Process a mapping object."""
    if isinstance(data, MutableMapping):
        return f"Mutable mapping with {len(data)} items"
    elif isinstance(data, Mapping):
        return f"Immutable mapping with {len(data)} items"
    else:
        return "Not a mapping"
