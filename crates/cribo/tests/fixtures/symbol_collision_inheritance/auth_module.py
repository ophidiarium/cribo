"""Module with classes that will have naming collision."""


class AuthBase:
    """Base authentication class."""

    def __init__(self):
        self.type = "base"

    def __repr__(self):
        return f"<{self.__class__.__name__}>"


class HTTPBasicAuth(AuthBase):
    """Basic HTTP authentication."""

    def __init__(self, username, password):
        super().__init__()
        self.username = username
        self.password = password
        self.type = "basic"

    def __repr__(self):
        return f"<HTTPBasicAuth user={self.username}>"


# This class inherits from HTTPBasicAuth defined in the same module
class HTTPProxyAuth(HTTPBasicAuth):
    """Proxy HTTP authentication - inherits from HTTPBasicAuth."""

    def __init__(self, username, password):
        super().__init__(username, password)
        self.type = "proxy"

    def __repr__(self):
        return f"<HTTPProxyAuth user={self.username}>"


def make_auth(auth_type, username, password):
    """Factory function that uses the classes."""
    if auth_type == "basic":
        return HTTPBasicAuth(username, password)
    elif auth_type == "proxy":
        return HTTPProxyAuth(username, password)
    else:
        return AuthBase()
