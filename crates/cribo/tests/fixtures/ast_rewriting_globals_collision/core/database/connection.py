"""Database connection module - imports only from core.utils to avoid circular dependencies"""

# Import from utils only (no circular dependency)
from core.utils.helpers import sanitize, format_result

# Global conflicts
process = None  # Will be set later
validate = lambda x: f"db_validate: {x}"  # Lambda instead of function
User = {"type": "database_user"}  # Dict instead of class


# Functions with common names
def process(data):
    """Process function in database module"""
    clean_data = sanitize(data)
    return f"db_process: {clean_data}"


def validate(data):
    """Validate function in database module"""
    return f"db_validate: {data} (overrides lambda)"


# Classes
class Connection:
    """Database connection class"""

    def __init__(self, db_name="default"):
        self.db_name = db_name
        self.connected = False

    def connect(self):
        self.connected = True
        return f"db_connection_to_{self.db_name}"

    def execute(self, query):
        if not self.connected:
            raise RuntimeError("Not connected")
        return format_result(f"Query: {query}")


class Logger:
    """Database logger - different from utils Logger"""

    def __init__(self, context):
        self.context = context

    def log(self, message):
        return f"DB_LOG[{self.context}]: {message}"


# Module-level functions
def create_connection(db_name):
    """Factory function"""
    conn = Connection(db_name)
    conn.connect()
    return conn
