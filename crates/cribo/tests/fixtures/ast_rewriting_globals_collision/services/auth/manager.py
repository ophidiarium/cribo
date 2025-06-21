"""Authentication manager - imports from core and models only, no circular dependencies"""

# Import from core and models (but not from modules that import this)
from core.database.connection import Connection as DBConnection
from core.utils.helpers import sanitize
from models.base import initialize as base_init

# Global conflicts
process = [1, 2, 3]  # List instead of function
validate = None  # Will be function
User = "auth_user_string"  # String before class definition
Logger = set()  # Set instead of class
Connection = {"auth": True}  # Dict instead of class


# Functions
def process(data):
    """Process function in auth module"""
    sanitized = sanitize(data)
    return f"auth_process: {sanitized}"


def validate(data):
    """Validate function in auth module"""
    if not isinstance(data, str):
        return f"auth_validate_failed: {data}"
    return f"auth_validate: {data}"


# Classes with conflicts
class User:
    """Auth user class"""

    def __init__(self, username):
        self.username = username
        self.connection = Connection  # References the dict, not a class

    def authenticate(self):
        return f"auth_user_{self.username}"


class Connection:
    """Auth connection - overrides the global dict"""

    def __init__(self):
        self.db_conn = DBConnection("auth_db")
        self.status = "auth_connection"

    def connect(self):
        db_result = self.db_conn.connect()
        return f"auth_wrapped_{db_result}"


class AuthManager:
    """Non-conflicting class name"""

    def __init__(self):
        self.users = {}
        self.base_initialized = base_init()

    def add_user(self, user):
        self.users[user.username] = user

    def process_auth(self, username, data):
        # Uses local process function
        return process(f"{username}:{data}")
