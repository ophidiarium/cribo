"""Sessions module with Session class."""


class Session:
    """A simple session class."""

    def __init__(self):
        self.headers = {}
        self.cookies = {}

    def request(self, method, url, **kwargs):
        """Make a request."""
        return f"Mock response for {method} {url}"

    def __enter__(self):
        """Context manager entry."""
        return self

    def __exit__(self, *args):
        """Context manager exit."""
        pass


def session():
    """Create a new session."""
    return Session()
