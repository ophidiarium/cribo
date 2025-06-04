"""
User model with role enumeration.
No naming conflicts with other modules.
"""

from enum import Enum
from typing import Optional


class UserRole(Enum):
    """User role enumeration."""

    ADMIN = "admin"
    USER = "user"
    GUEST = "guest"


class User:
    """Simple user model."""

    def __init__(self, name: str, email: str, role: UserRole = UserRole.USER):
        self.name = name
        self.email = email
        self.role = role
        self.active = True

    def __str__(self) -> str:
        return f"User(name='{self.name}', email='{self.email}', role={self.role.value})"

    def __repr__(self) -> str:
        return self.__str__()

    def activate(self) -> None:
        """Activate the user."""
        self.active = True

    def deactivate(self) -> None:
        """Deactivate the user."""
        self.active = False


# Module-level constant
DEFAULT_ROLE = UserRole.USER
