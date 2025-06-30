"""Another module that exports HTTPBasicAuth to create a collision."""


# Different implementation with same name
class HTTPBasicAuth:
    """Different implementation of HTTPBasicAuth to create collision."""

    def __init__(self, token):
        self.token = token
        self.type = "token"

    def __repr__(self):
        return f"<HTTPBasicAuth(token) token={self.token[:4]}...>"


# Export it so it collides when both modules are imported
__all__ = ["HTTPBasicAuth"]
