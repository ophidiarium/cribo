"""Core utility helpers module - NO external dependencies to avoid circular imports"""

# Global variables
process_count = 0
validate_cache = {}
Logger = str  # Type alias conflict
Connection = "not_a_class"  # String instead of class


# Functions with common names
def process(data):
    """Process function in utils module"""
    global process_count
    process_count += 1
    return f"utils_process: {data} (#{process_count})"


def validate(data):
    """Validate function in utils module"""
    if data in validate_cache:
        return validate_cache[data]
    result = f"utils_validate: {data}"
    validate_cache[data] = result
    return result


# Classes
class Logger:
    """Logger class in utils module"""

    def __init__(self, name):
        self.name = name
        self.messages = []

    def log(self, message):
        self.messages.append(f"[{self.name}] {message}")

    def get_message(self):
        return f"utils_logger_{self.name}"


class Connection:
    """Connection class in utils - different from database Connection"""

    def __init__(self):
        self.type = "utility_connection"

    def connect(self):
        return "utils_connection_established"


# Helper functions
def sanitize(text):
    """Utility function without conflicts"""
    return text.strip().lower()


def format_result(result):
    """Another utility function"""
    return f"formatted: {result}"
