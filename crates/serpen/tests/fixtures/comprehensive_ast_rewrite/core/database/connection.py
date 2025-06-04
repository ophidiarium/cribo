"""
Database connection module with naming conflicts
"""

from ...models.user import process_user  # Deep relative import
from ..utils.helpers import validate as helper_validate  # Relative import

# Global variables that conflict with other modules
result = []
connection = None


class Connection:
    """Database connection class"""

    def __init__(self, host="localhost", port=5432):
        self.host = host
        self.port = port
        self.connected = False
        # Variable name conflicts
        self.result = None
        self.process = self._internal_process

    def _internal_process(self, query):
        return f"db_internal: {query}"

    def connect(self):
        global connection
        self.connected = True
        connection = self
        return f"Connected to {self.host}:{self.port}"


def process(data):
    """Database process function - conflicts with other process functions"""
    global result

    # Use helper validation with conflict resolution
    validated = helper_validate(data)

    # Process user data with relative import
    user_result = process_user(data)

    # Build result with local processing
    processed = {"db_process": True, "data": validated, "user_processing": user_result, "timestamp": "2024-01-01"}

    result.append(processed)
    return f"db_processed: {data}"


def validate(data):
    """Database validate function - another conflict"""
    if not data:
        return False
    return f"db_valid: {data}"


# Function with same name as class method
def connect():
    """Module-level connect function"""
    global connection
    if connection is None:
        connection = Connection()
    return connection.connect()


# Variable reuse for different purposes
process = process  # Self-reference for alias testing
