"""Module with side effects that requires wrapping."""

import logging

# This side effect forces the module to be wrapped
print("Module loaded - this is a side effect")


def get_logger(name):
    """Get a logger instance using the logging module."""
    # This will fail if logging is not available in the wrapper function scope
    return logging.getLogger(name)
