# Exception definitions that should not be tree-shaken


class CustomError(Exception):
    """Custom exception that is used by utils module."""

    pass


class UnusedError(Exception):
    """This exception is never used and should be tree-shaken."""

    pass
