"""Models package."""

from .user import User, UserManager

# Self-references in package init (should be removed)
User = User  # Should be removed
UserManager = UserManager  # Should be removed
