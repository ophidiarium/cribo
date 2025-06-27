"""
Module B with a process_data function.
This will conflict with module_a.process_data when bundled.
"""


def process_data(input_data: str) -> str:
    """Process data in module B's way."""
    return f"Module B processed: {input_data.lower()}"


def another_helper() -> str:
    """A helper function unique to module B."""
    return "Module B helper"


# Module-level variable
MODULE_NAME = "module_b"
