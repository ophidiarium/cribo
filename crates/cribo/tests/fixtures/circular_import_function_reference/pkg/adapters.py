"""Adapters module that imports a function from auth."""

from .auth import _basic_auth_str  # Import the function directly
from .models import BaseModel


class Adapter(BaseModel):
    """Adapter class that uses auth functionality."""

    def __init__(self):
        super().__init__()
        self.auth_header = None

    def use_auth(self, username, password):
        """Use the imported auth function."""
        # This should fail if _basic_auth_str is not available
        self.auth_header = _basic_auth_str(username, password)
        print(f"Auth header set: {self.auth_header}")
        return self.auth_header
