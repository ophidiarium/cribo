"""
Authentication manager with complex naming conflicts
"""

from typing import Optional, Dict, Any
from ...core.database.connection import Connection as DBConnection  # Cross-package import
from ...models import base  # Cross-package relative import

# Global conflicts
result = "auth_result"
validate = lambda x: f"auth_lambda_validate: {x}"  # Lambda conflicts with function names


class User:
    """Auth User class - conflicts with other User classes/variables"""

    def __init__(self, username: str, password: str):
        self.username = username
        self.password = password
        self.authenticated = False
        # Property conflicts with global/import names
        self.result = None
        self.connection = None

    def authenticate(self) -> bool:
        """Authenticate user with name conflicts"""
        # Local variable conflicts
        validate = self._internal_validate  # Shadows global validate
        result = validate(self.password)  # Shadows global result

        self.authenticated = result
        self.result = f"auth_user_{self.username}_{result}"
        return result

    def _internal_validate(self, password: str) -> bool:
        return len(password) >= 4

    def connect(self) -> str:
        """Method with same name as global functions"""
        self.connection = DBConnection()
        return f"User {self.username} connected"


class Connection:
    """Auth connection class - conflicts with DB Connection"""

    def __init__(self, auth_type: str = "oauth"):
        self.auth_type = auth_type
        self.users = []

    def add_user(self, User: "User") -> None:  # Parameter shadows class name
        """Add user with parameter name conflict"""
        self.users.append(User)  # Using parameter, not class

    def process(self, User: str) -> str:  # Parameter name conflicts
        """Process with parameter name conflicts"""
        return f"auth_connection_process: {User}"


def process(data: Any) -> str:
    """Auth process function - major conflict"""
    global result

    # Use base module from relative import
    base_init = base.initialize()

    # Complex processing with conflicts
    if isinstance(data, str):
        # Local validate shadows global validate
        validate = lambda x: x.startswith("auth_")
        validated = validate(data)
        processed = f"auth_str_{data}" if validated else f"invalid_auth_{data}"
    else:
        processed = f"auth_other_{data}"

    # Update global result
    result = f"{result}_processed"

    return f"auth_processed: {processed}, base: {base_init}"


def validate(data: Any) -> str:
    """Auth validate function - conflicts with other validate functions"""
    if not data:
        return "auth_invalid"

    # Use global validate lambda (fixed syntax)
    global_validate = globals().get("validate", lambda x: f"fallback_{x}")
    lambda_result = global_validate(data) if callable(global_validate) else str(data)

    return f"auth_valid: {data}, lambda: {lambda_result}"


def connect(User: Optional["User"] = None) -> Connection:
    """Connect function with parameter conflict"""
    connection = Connection("auth_manager")
    if User:  # Using parameter
        connection.add_user(User)
    return connection


# Complex class with method name conflicts
class AuthManager:
    """Manager class with extensive conflicts"""

    def __init__(self):
        self.connections = []
        self.users = []
        # Instance variable conflicts
        self.process = self._manager_process
        self.validate = self._manager_validate
        self.User = None  # Instance var conflicts with class name

    def _manager_process(self, data: Any) -> str:
        return f"manager_process: {data}"

    def _manager_validate(self, data: Any) -> bool:
        return data is not None

    def add_user(self, username: str, password: str) -> "User":
        """Method that creates User with local scope conflicts"""
        User = globals()["User"]  # Get class from globals
        user = User(username, password)  # Local var shadows class name
        self.users.append(user)
        self.User = user  # Instance var assignment
        return user

    def process_all(self) -> Dict[str, Any]:
        """Method using conflicting names throughout"""
        result = []  # Local result shadows global

        for User in self.users:  # Loop var shadows class name
            user_result = process(User.username)  # Module function
            validate_result = validate(User.password)  # Module function

            # Complex nested conflicts
            connection = connect(User)  # Module function with User param
            connection_process = connection.process(User.username)

            result.append({"user": User.username, "process": user_result, "validate": validate_result, "connection": connection_process})

        return {"manager_results": result}


# Global function/variable assignments with conflicts
process = process  # Self-reference
validate = validate  # Self-reference
User = User  # Self-reference
