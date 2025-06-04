"""
Utility helpers module with extensive naming conflicts
"""

import os
from typing import Any, Dict, List

# Global variable conflicts
result = 0
User = "helper_user_string"  # String that conflicts with User classes


class Logger:
    """Utility logger class - conflicts with other Logger classes"""

    def __init__(self, name: str):
        self.name = name
        self.messages: List[str] = []
        # Internal conflict with global User
        self.User = User  # Reference to global string

    def log(self, message: str) -> None:
        self.messages.append(f"[{self.name}] {message}")

    def get_message(self) -> str:
        return f"Utils Logger: {self.name}"

    def process(self, data: Any) -> str:
        """Method with same name as module functions"""
        return f"logger_process: {data}"


def process(data: Any) -> str:
    """Utility process function - major conflict with other process functions"""
    global result

    # Complex processing with type checking
    if isinstance(data, str):
        processed = data.upper()
    elif isinstance(data, (int, float)):
        processed = data * 2
    elif isinstance(data, dict):
        processed = {k: f"util_{v}" for k, v in data.items()}
    else:
        processed = str(data)

    result += 1
    return f"util_processed: {processed}"


def validate(data: Any) -> bool:
    """Utility validate function - conflicts with validate in other modules"""
    if data is None:
        return False

    if isinstance(data, str):
        return len(data) > 0
    elif isinstance(data, (list, dict)):
        return len(data) > 0
    elif isinstance(data, (int, float)):
        return data >= 0

    return True


class Connection:
    """Utility connection class - name conflict with database Connection"""

    def __init__(self, connection_type: str = "utility"):
        self.connection_type = connection_type
        self.active = False

    def connect(self) -> str:
        self.active = True
        return f"Utility connection: {self.connection_type}"


def connect() -> Connection:
    """Utility connect function"""
    return Connection("helper")


# Complex function with multiple parameter conflicts
def process_with_conflicts(
    data: Any,
    User: str = "param_user",  # Parameter name conflicts with global User
    result: int = 100,  # Parameter name conflicts with global result
    Logger: Any = None,  # Parameter name conflicts with Logger class
) -> Dict[str, Any]:
    """Function with parameter names that conflict with globals and imports"""

    # Local variable conflicts
    connection = connect()
    validate_result = validate(data)

    # Using parameters with same names as globals/classes
    local_result = {
        "data": data,
        "user_param": User,  # Using parameter, not global
        "result_param": result,  # Using parameter, not global
        "logger_param": Logger,  # Using parameter, not class
        "validation": validate_result,
        "connection_type": connection.connection_type,
    }

    return local_result


# Variable assignment using function names
validate = validate  # Self-reference for testing
process = process  # Self-reference for testing
