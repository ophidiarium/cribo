"""
Models module with User and Product classes.
These will conflict with entities.py when bundled.
"""


class User:
    """User model for data persistence."""

    def __init__(self, name: str, email: str):
        self.name = name
        self.email = email
        self.id = None

    def __str__(self) -> str:
        return f"Model User(name='{self.name}', email='{self.email}')"


class Product:
    """Product model for catalog."""

    def __init__(self, name: str, price: float):
        self.name = name
        self.price = price
        self.id = None

    def __str__(self) -> str:
        return f"Model Product(name='{self.name}', price={self.price})"


# Module-level function
def create_user(name: str, email: str) -> User:
    """Factory function for creating users."""
    return User(name, email)
