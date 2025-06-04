"""
Entities module with User and Product classes.
These will conflict with models.py when bundled.
"""


class User:
    """User entity for business logic."""

    def __init__(self, name: str, age: int):
        self.name = name
        self.age = age
        self.active = True

    def __str__(self) -> str:
        return f"Entity User(name='{self.name}', age={self.age})"


class Product:
    """Product entity for business logic."""

    def __init__(self, sku: str, name: str):
        self.sku = sku
        self.name = name
        self.available = True

    def __str__(self) -> str:
        return f"Entity Product(sku='{self.sku}', name='{self.name}')"


# Module-level function
def create_product(sku: str, name: str) -> Product:
    """Factory function for creating products."""
    return Product(sku, name)
