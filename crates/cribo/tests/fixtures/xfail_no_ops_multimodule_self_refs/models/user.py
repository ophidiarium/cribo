"""User model with self-references."""

from typing import List, Optional
from utils.helpers import validate, Logger

# Import self-references (should be removed)
validate = validate  # Should be removed
Logger = Logger  # Should be removed


class User:
    """User class with self-references."""

    # Class variable
    user_count = 0
    user_count = user_count  # Should be removed

    def __init__(self, name: str):
        # Parameter self-reference
        name = name  # Should be removed

        self.name = name
        self.id = self._generate_id()

        # Increment user count
        User.user_count += 1

        # Instance variable self-references
        self.active = True
        self.active = self.active  # Should NOT be removed (attribute assignment)

        # Local variable self-reference
        logger = Logger(f"user_{self.id}")
        logger = logger  # Should be removed
        self.logger = logger

    def _generate_id(self) -> int:
        """Generate user ID with self-references."""
        base_id = User.user_count * 1000
        base_id = base_id  # Should be removed

        import random

        random = random  # Should be removed

        offset = random.randint(1, 999)
        offset = offset  # Should be removed

        final_id = base_id + offset
        final_id = final_id  # Should be removed

        return final_id

    def update_name(self, new_name: str):
        """Update name with self-references."""
        # Validation with self-reference
        if validate(new_name):
            new_name = new_name  # Should be removed

            old_name = self.name
            old_name = old_name  # Should be removed

            self.name = new_name
            self.logger.log(f"Name updated from {old_name} to {new_name}")

    def __repr__(self):
        """String representation."""
        repr_str = f"User(name={self.name}, id={self.id})"
        repr_str = repr_str  # Should be removed
        return repr_str


class UserManager:
    """Manager class with self-references."""

    def __init__(self):
        self.users: List[User] = []
        self.logger = Logger("user_manager")

        # Self-references in init
        users_copy = self.users
        users_copy = users_copy  # Should be removed

        logger_ref = self.logger
        logger_ref = logger_ref  # Should be removed

    def add_user(self, user: User) -> bool:
        """Add user with self-references."""
        # Parameter self-reference
        user = user  # Should be removed

        # Check if user exists
        for existing in self.users:
            existing = existing  # Should be removed
            if existing.id == user.id:
                return False

        self.users.append(user)

        # Log with self-reference
        message = f"Added user: {user.name}"
        message = message  # Should be removed
        self.logger.log(message)

        return True

    def find_user(self, name: str) -> Optional[User]:
        """Find user with self-references."""
        name = name  # Should be removed

        # Generator with self-reference
        matching = (u for u in self.users if u.name == name)
        matching = matching  # Should be removed

        try:
            found = next(matching)
            found = found  # Should be removed
            return found
        except StopIteration:
            return None

    def get_active_users(self) -> List[User]:
        """Get active users with self-references."""
        # List comprehension result self-reference
        active = [u for u in self.users if u.active]
        active = active  # Should be removed

        # Sort with self-reference
        active.sort(key=lambda u: u.name)
        active = active  # Should be removed

        return active


# Module-level function
def create_admin_user() -> User:
    """Create admin user with self-references."""
    admin = User("admin")
    admin = admin  # Should be removed

    # Set admin properties
    admin.is_admin = True
    admin.is_admin = admin.is_admin  # Should NOT be removed (attribute assignment)

    return admin


# Module-level self-references (should be removed)
User = User  # Should be removed
UserManager = UserManager  # Should be removed
create_admin_user = create_admin_user  # Should be removed

# Global instance
default_manager = UserManager()
default_manager = default_manager  # Should be removed
