"""Parent package that imports from its child module."""

# Side effect to track module initialization order
print("Initializing foo package")

# This import creates a circular dependency because:
# - foo.boo will need foo to be initialized first (Python semantics)
# - But foo needs something from foo.boo
from .boo import helper_function


# Use the imported function
def package_level_function(x):
    """A function at the package level that uses the imported helper."""
    return helper_function(x) + " (from package)"


# Export for external use
__all__ = ["package_level_function", "helper_function"]
