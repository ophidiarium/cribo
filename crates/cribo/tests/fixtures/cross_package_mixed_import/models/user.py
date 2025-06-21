"""User model providing cross-package functionality."""


def process_user(name: str) -> str:
    """Process a user-related string.

    This function is imported by core.database.connection,
    demonstrating a cross-package absolute import.
    """
    return f"user_{name}_processed"
