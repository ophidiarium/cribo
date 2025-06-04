#!/usr/bin/env python3
"""
Comprehensive AST rewriter test fixture - Main entry point
This module demonstrates complex naming conflicts and import scenarios
"""

# Import conflicts: 'process' function exists in multiple modules
from core.database.connection import process as db_process
from core.utils.helpers import process, Logger as UtilLogger, validate
from services.auth.manager import process as auth_process, User, validate as auth_validate
from models.user import User as UserModel, process_user, Logger
from .models import base  # Relative import

# Variable name conflicts
result = 42
connection = None
Logger = "string_logger"  # Conflicts with imported Logger classes


# Function name conflicts with imports
def validate(data):
    """This validate function conflicts with imported validate functions"""
    return f"main_validate: {data}"


def process():
    """This process function conflicts with multiple imported process functions"""
    return "main_process"


# Class name conflicts
class User:
    """This User class conflicts with imported User classes"""

    def __init__(self, name):
        self.name = name
        # Variable conflict: 'result' used in different contexts
        self.result = self._process_name(name)

    def _process_name(self, name):
        return f"main_user: {name}"


class Connection:
    """Connection class that conflicts with database connection"""

    def __init__(self):
        self.status = "disconnected"

    def connect(self):
        global connection
        connection = self
        return "main_connection_established"


def main():
    """Main function demonstrating all the conflicts in action"""
    # Using imported functions with aliases
    db_result = db_process("database_data")
    util_result = process("utility_data")  # Imported function
    auth_result = auth_process("auth_data")

    # Using imported classes with conflicts
    util_logger = UtilLogger("util")
    model_logger = Logger("model")  # Different Logger class

    # Using imported User classes vs local User class
    auth_user = User("auth_type")  # Local User class
    model_user = UserModel("model_type")  # Imported User class
    service_user = User("service_type", "password")  # Imported User class from auth

    # Using local functions that conflict with imports
    local_validate_result = validate("local_data")  # Local function
    auth_validate_result = auth_validate("auth_data")  # Imported function
    util_validate_result = validate("util_data")  # Imported from utils

    # Variable name reuse and conflicts
    result = db_result + util_result + auth_result  # Shadows global result

    # Using relative imports
    base_result = base.initialize()

    # Complex expression with multiple conflicts
    final_result = {
        "process_results": [db_result, util_result, auth_result],
        "validation_results": [local_validate_result, auth_validate_result, util_validate_result],
        "user_types": [auth_user.name, model_user.name, service_user.username],
        "logger_messages": [util_logger.get_message(), model_logger.get_message()],
        "base_init": base_result,
        "total": result + globals()["result"],  # Global vs local result conflict
    }

    return final_result


if __name__ == "__main__":
    # More conflicts in main execution
    connection = Connection()
    connection.connect()

    results = main()
    print("Comprehensive AST rewriter test completed")
    print(f"Final results: {results}")
