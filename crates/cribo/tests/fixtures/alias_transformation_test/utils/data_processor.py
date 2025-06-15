"""
Data processing utilities module.
Used to test from-import alias transformations.
"""


def process_data(data_list):
    """Process a list of data by doubling each element."""
    return [x * 2 for x in data_list]


def format_output(processed_data):
    """Format processed data as a comma-separated string."""
    return ", ".join(map(str, processed_data))


def unaliased_function():
    """This function is not imported with an alias."""
    return "Not imported with alias"
