"""First auth module with HTTPBasicAuth."""


class HTTPBasicAuth:
    """Basic HTTP authentication."""

    def __init__(self, username, password):
        self.username = username
        self.password = password

    def __repr__(self):
        return f"<HTTPBasicAuth user={self.username}>"
