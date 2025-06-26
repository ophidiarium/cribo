"""User models module - imports from core and base, no circular dependencies"""

# Import from core (not from services or models that might import this)
from core.utils.helpers import process as util_process, Logger as UtilLogger

# Import from sibling module
from .base import Logger as BaseLogger

# Global conflicts
process = "process_string"  # String instead of function
validate = {"action": "validate"}  # Dict instead of function
User = None  # Will be class later
Connection = type("Connection", (), {"source": "models.user"})


# Functions with conflicts
def process(data):
    """Process function in user module"""
    return f"user_process: {data}"


def process_user(user_data):
    """Process user-specific data"""
    util_result = util_process(user_data)
    return f"process_user: {user_data} -> {util_result}"


def validate(data):
    """Validate function in user module"""
    return f"user_validate: {data}"


# Classes
class User:
    """User model class"""

    def __init__(self, name):
        self.name = name
        self.logger = Logger("user")  # Uses local Logger class

    def process(self):
        # Method that shares name with module functions
        return f"User.process: {self.name}"


class Logger:
    """User module logger - different from utils and base Logger"""

    def __init__(self, context):
        self.context = context
        self.base_logger = BaseLogger(context)
        self.util_logger = UtilLogger(context)

    def get_message(self):
        return f"user_logger_{self.context}"

    def log_all(self):
        return [
            self.get_message(),
            self.base_logger.get_message(),
            self.util_logger.get_message(),
        ]


# Helper class
class UserValidator:
    """Non-conflicting class name"""

    def __init__(self):
        self.rules = []

    def add_rule(self, rule):
        self.rules.append(rule)
