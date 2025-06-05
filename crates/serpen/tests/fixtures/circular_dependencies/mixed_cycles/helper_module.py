"""Helper module that creates resolvable cycle with function_module."""


def transform(value):
    """Transform a value using another function."""
    from function_module import utility_function

    prefix = utility_function()
    return value + len(prefix)
