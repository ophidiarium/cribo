#!/usr/bin/env python3
"""Test fixture for self-reference assignments across multiple modules."""

# Import various modules
from utils.helpers import process_data, validate, Logger
from core.processor import DataProcessor, transform
from models.user import User, UserManager
import utils.constants as constants
import core
import models

# Module-level self-references after imports (should be removed)
process_data = process_data  # Should be removed
validate = validate  # Should be removed
Logger = Logger  # Should be removed
DataProcessor = DataProcessor  # Should be removed
transform = transform  # Should be removed
User = User  # Should be removed
constants = constants  # Should be removed
core = core  # Should be removed
models = models  # Should be removed

# Local variables
result = None
data = []

# More self-references (should be removed)
result = result  # Should be removed
data = data  # Should be removed


def main():
    """Main function with self-references in function scope."""
    # Function-level imports
    from utils.helpers import helper_function
    from core.config import CONFIG

    # Self-references in function scope (should be removed)
    helper_function = helper_function  # Should be removed
    CONFIG = CONFIG  # Should be removed

    # Create instances
    logger = Logger("main")
    processor = DataProcessor()
    user_manager = UserManager()

    # Self-references with instances (should be removed)
    logger = logger  # Should be removed
    processor = processor  # Should be removed
    user_manager = user_manager  # Should be removed

    # Process data
    raw_data = [1, 2, 3, 4, 5]
    raw_data = raw_data  # Should be removed

    # Use imported functions
    if validate(raw_data):
        processed = process_data(raw_data)
        transformed = transform(processed)

        # Self-references in nested scope (should be removed)
        processed = processed  # Should be removed
        transformed = transformed  # Should be removed

        # Create user
        user = User("test_user")
        user = user  # Should be removed

        user_manager.add_user(user)

        # Use constants
        max_value = constants.MAX_VALUE
        max_value = max_value  # Should be removed

        logger.log(f"Processed {len(transformed)} items, max value: {max_value}")

        return transformed

    return None


class MainController:
    """Class with self-references in methods."""

    def __init__(self):
        self.processor = DataProcessor()
        self.logger = Logger("controller")

        # Self-references in __init__ (should be removed)
        processor = DataProcessor()
        processor = processor  # Should be removed

    def run(self):
        """Method with various self-references."""
        # Import in method
        from utils.helpers import get_config

        # Self-reference of imported function (should be removed)
        get_config = get_config  # Should be removed

        config = get_config()
        config = config  # Should be removed

        # Use instance attributes
        result = self.processor.process([1, 2, 3])
        result = result  # Should be removed

        # Complex expression that evaluates to self-reference
        temp = result if result else result
        temp = temp  # Should be removed

        return result


# Self-references at module level after class definition
MainController = MainController  # Should be removed
main = main  # Should be removed

if __name__ == "__main__":
    # Self-references in main block
    controller = MainController()
    controller = controller  # Should be removed

    results = main()
    results = results  # Should be removed

    if results:
        print(f"Success: {len(results)} items processed")
    else:
        print("No results")

    # Final self-references
    controller.run()
    __name__ = __name__  # Should be removed (though unusual)
