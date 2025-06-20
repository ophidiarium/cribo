"""Helper utilities with self-reference patterns."""

import os
import sys
from typing import Any, List, Dict

# Self-references with stdlib imports (should be removed)
os = os  # Should be removed
sys = sys  # Should be removed

# Constants with self-references
DEFAULT_TIMEOUT = 30
DEFAULT_RETRIES = 3

DEFAULT_TIMEOUT = DEFAULT_TIMEOUT  # Should be removed
DEFAULT_RETRIES = DEFAULT_RETRIES  # Should be removed


class Logger:
    """Logger class with self-references."""

    # Class attribute self-reference
    default_level = "INFO"
    default_level = default_level  # Should be removed

    def __init__(self, name: str):
        self.name = name
        self.level = self.default_level

        # Self-references in init (should be removed)
        name = name  # Should be removed
        self.level = self.level  # Should NOT be removed (attribute assignment)

    def log(self, message: str):
        """Log a message."""
        # Self-reference in method
        formatted = f"[{self.name}] {message}"
        formatted = formatted  # Should be removed
        print(formatted)


def process_data(data: List[Any]) -> List[Any]:
    """Process data with self-references."""
    # Parameter self-reference (should be removed)
    data = data  # Should be removed

    # Local variable self-references
    result = []
    count = 0

    result = result  # Should be removed
    count = count  # Should be removed

    for item in data:
        # Self-reference in loop (should be removed)
        item = item  # Should be removed

        processed = item * 2 if isinstance(item, (int, float)) else str(item)
        processed = processed  # Should be removed

        result.append(processed)
        count += 1

    # Self-reference before return
    result = result  # Should be removed
    return result


def validate(data: Any) -> bool:
    """Validate data with self-references."""
    # Early return with self-reference
    if data is None:
        data = data  # Should be removed (unreachable but parser might not know)
        return False

    # Nested function with self-references
    def is_valid_item(item):
        # Self-reference in nested function
        item = item  # Should be removed
        return item is not None

    # Self-reference of nested function
    is_valid_item = is_valid_item  # Should be removed

    if isinstance(data, list):
        valid = all(is_valid_item(item) for item in data)
        valid = valid  # Should be removed
        return valid

    return True


def helper_function():
    """Helper function for imports."""
    value = 42
    value = value  # Should be removed
    return value


def get_config() -> Dict[str, Any]:
    """Get configuration with self-references."""
    config = {"timeout": DEFAULT_TIMEOUT, "retries": DEFAULT_RETRIES, "debug": False}

    # Self-reference of dict (should be removed)
    config = config  # Should be removed

    # Self-reference in dict comprehension result
    filtered = {k: v for k, v in config.items() if v is not None}
    filtered = filtered  # Should be removed

    return filtered


# Module-level function self-references (should be removed)
process_data = process_data  # Should be removed
validate = validate  # Should be removed
helper_function = helper_function  # Should be removed
get_config = get_config  # Should be removed
Logger = Logger  # Should be removed

# Complex self-reference patterns
_private_var = 100
_private_var = _private_var  # Should be removed

# Self-reference in conditional
if True:
    conditional_var = 200
    conditional_var = conditional_var  # Should be removed
