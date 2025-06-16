"""User class module with circular dependency."""

from admin_class import Admin


class User:
    """User class that references Admin class."""

    def __init__(self, name):
        self.name = name
        self.admin_reference = Admin

    def promote_to_admin(self):
        """Promote user to admin."""
        return self.admin_reference(self.name)
