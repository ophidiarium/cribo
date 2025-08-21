"""Helper module to test import aliasing that shadows stdlib names."""


def process_data(data):
    """Process data in a custom way."""
    if isinstance(data, dict):
        return {"processed": True, "original": data}
    return {"processed": True, "original": str(data)}
