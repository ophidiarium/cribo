"""
Module A with a process_data function.
This will conflict with module_b.process_data when bundled.
"""


def process_data(input_data: str) -> str:
    """Process data in module A's way."""
    return f"Module A processed: {input_data.upper()}"


def helper_function() -> str:
    """A helper function unique to module A."""
    return "Module A helper"


# Module-level variable
MODULE_NAME = "module_a"
