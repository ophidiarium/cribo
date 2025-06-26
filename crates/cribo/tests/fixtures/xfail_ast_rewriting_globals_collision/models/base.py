"""Base models module - imports only from core to avoid circular dependencies"""

# Import from core modules only
from core.utils.helpers import validate as util_validate

# Global conflicts
process = lambda x: f"base_process: {x}"
validate = 123  # Integer instead of function
Logger = None  # Will be defined as class later
Connection = []  # List instead of class


# Functions
def process(data):
    """Process function in base module"""
    return f"base_process: {data} (overrides lambda)"


def validate(data):
    """Validate function that uses imported validate"""
    base_check = f"base_validate: {data}"
    util_check = util_validate(data)
    return f"{base_check} + {util_check}"


# Classes
class Logger:
    """Base logger class"""

    def __init__(self, prefix="BASE"):
        self.prefix = prefix

    def get_message(self):
        return f"base_logger_{self.prefix}"


# Module initialization
def initialize():
    """Initialize base module"""
    global Connection
    Connection = type("Connection", (), {"type": "base_connection"})
    return "base_initialized"
