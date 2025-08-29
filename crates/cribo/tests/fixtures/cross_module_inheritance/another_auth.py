"""Another auth module with HTTPBasicAuth to create collision."""


class HTTPBasicAuth:
    """Different HTTPBasicAuth implementation."""

    def __init__(self, token):
        self.token = token

    def __repr__(self):
        return f"<TokenAuth token={self.token[:4]}...>"
