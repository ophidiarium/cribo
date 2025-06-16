"""Module with resolvable function-level circular dependency."""


def process_data(value):
    """Process data using helper from another module."""
    from helper_module import transform

    return transform(value) * 2


def utility_function():
    """Another function that doesn't create a cycle."""
    return "utility"
