"""structures.py - Contains data structures."""

from compat import MutableMapping


class CaseInsensitiveDict(MutableMapping):
    """A case-insensitive dict-like object."""

    def __init__(self):
        self._store = {}

    def __setitem__(self, key, value):
        self._store[key.lower()] = value

    def __getitem__(self, key):
        return self._store[key.lower()]

    def __delitem__(self, key):
        del self._store[key.lower()]

    def __iter__(self):
        return iter(self._store)

    def __len__(self):
        return len(self._store)
