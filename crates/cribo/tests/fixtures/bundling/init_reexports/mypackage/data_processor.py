"""Data processing module."""

def process_data(data):
    """Process the input data."""
    processed = data.copy()
    processed["processed"] = True
    return processed