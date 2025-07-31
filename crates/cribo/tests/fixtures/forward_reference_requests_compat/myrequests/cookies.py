"""Cookies module that uses compat's MutableMapping in class inheritance."""

from .compat import cookielib, MutableMapping, urlparse


# This class inherits from both cookielib.CookieJar and MutableMapping
class CookieJar(cookielib.CookieJar, MutableMapping):
    """A cookie jar that also implements the MutableMapping interface."""

    def __init__(self):
        super().__init__()
        self._cookies = {}

    def __getitem__(self, name):
        return self._cookies[name]

    def __setitem__(self, name, value):
        self._cookies[name] = value

    def __delitem__(self, name):
        del self._cookies[name]

    def __iter__(self):
        return iter(self._cookies)

    def __len__(self):
        return len(self._cookies)

    def get(self, name, default=None):
        return self._cookies.get(name, default)

    def set(self, name, value):
        self._cookies[name] = value
