"""Module with function defined in try-except-else block."""

try:
    # Try to import something that may fail
    import nonexistent_module

    HAS_MODULE = True
except ImportError:
    # Define a fallback function in the except block
    def get_function():
        return "fallback function"

    HAS_MODULE = False
else:
    # Define the real function in the else block
    def get_function():
        return "real function with module"

# This function should be accessible from other modules
# regardless of which branch was taken
