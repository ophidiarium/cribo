"""Proxy module that inherits from HTTPBasicAuth in a different module.

This is the key part - proxy.py imports HTTPBasicAuth from auth.py
and creates a class that inherits from it. When HTTPBasicAuth gets
renamed due to collision, this inheritance needs to be updated.
"""

from auth import HTTPBasicAuth


class HTTPProxyAuth(HTTPBasicAuth):
    """Proxy authentication that inherits from HTTPBasicAuth."""

    def __init__(self, username, password):
        super().__init__(username, password)
        self.proxy_info = "proxy"

    def __repr__(self):
        return f"<HTTPProxyAuth user={self.username}>"
