"""Admin class module with circular dependency."""

from user_class import User


class Admin(User):
    """Admin class that inherits from User."""

    def __init__(self, name):
        super().__init__(name)
        self.is_admin = True

    def demote_to_user(self):
        """Demote admin to regular user."""
        return User(self.name)
