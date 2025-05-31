"""User model for the test project."""

from dataclasses import dataclass
from typing import Optional

@dataclass
class User:
    """A simple user model."""
    name: str
    age: int
    email: Optional[str] = None
    
    def is_adult(self) -> bool:
        """Check if the user is an adult."""
        return self.age >= 18
    
    def get_display_name(self) -> str:
        """Get a display name for the user."""
        if self.email:
            return f"{self.name} <{self.email}>"
        return self.name
