"""Simple helper module used through a module-object alias."""

MESSAGE = "module namespace preserved"


def helper():
    """Keep one callable export on the namespace for regression coverage."""
    return MESSAGE.upper()
