"""
Helper utilities module.
Used to test mixed from-import scenarios (some aliased, some not).
"""


def helper_func(input_str):
    """A helper function that processes a string."""
    return f"Processed: {input_str.upper()}"


def debug_print(message):
    """Debug printing function (will be imported with alias)."""
    print(f"[DEBUG] {message}")


def another_helper():
    """Another helper function (not imported)."""
    return "Another helper result"
