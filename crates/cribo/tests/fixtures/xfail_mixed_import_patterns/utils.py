"""
Utility module with no dependencies on other modules in this package.
This can be safely imported at module level by everyone.
"""


def format_message(message):
    """Format a message with decorative borders"""
    return f">>> {message} <<<"


def get_timestamp():
    """Get current timestamp for logging"""
    # Return fixed timestamp for deterministic output
    return "00:00:00"
