#!/usr/bin/env python3
"""
Test script demonstrating __init__.py re-export preservation.

This fixture tests that imports in __init__.py files are preserved even if they
appear "unused" within that file, as they are typically re-exports for the package interface.
"""

from mypackage import format_data, process_data, config
from mypackage.utils import helper_function


def main():
    """Main function demonstrating usage of re-exported functions."""
    data = {"name": "test", "value": 42}

    # Use functions re-exported through __init__.py
    processed = process_data(data)
    formatted = format_data(processed)

    # Use utility function
    result = helper_function(formatted)

    # Use config
    if config.DEBUG:
        print(f"Debug: {result}")
    else:
        print(result)


if __name__ == "__main__":
    main()
