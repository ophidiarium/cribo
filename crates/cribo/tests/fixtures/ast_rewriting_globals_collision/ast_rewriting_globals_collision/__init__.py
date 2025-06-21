# Nested package with same name as parent
# This creates additional complexity for the bundler

from core.utils import process as nested_process
from models.user import User as NestedUser

# Even more conflicts
Logger = lambda msg: print(f"nested_logger: {msg}")
validate = [1, 2, 3, 4, 5]


# Function that uses various conflicting names
def demonstrate_conflicts():
    """Show how conflicts are resolved in nested package"""
    return {
        "nested_process": nested_process("nested_data"),
        "nested_user": NestedUser("nested").name,
        "logger_type": type(Logger).__name__,
        "validate_value": validate,
    }
