"""Models module containing classes that will be shadowed by parameters."""


class User:
    """User class that will be shadowed by function parameters."""

    def __init__(self, name: str):
        self.name = name
        self.id = hash(name) % 10000

    def __repr__(self):
        return f"User(name={self.name!r}, id={self.id})"


class Connection:
    """Connection class that will be shadowed by function parameters."""

    def __init__(self, host: str, port: int):
        self.host = host
        self.port = port
        self.connected = False

    def connect(self):
        """Simulate connection."""
        self.connected = True
        return f"Connected to {self.host}:{self.port}"

    def __repr__(self):
        return f"Connection(host={self.host!r}, port={self.port}, connected={self.connected})"
