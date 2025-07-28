"""Core module that creates circular dependency."""

# Import the entire package to create circular dependency
import mypackage


def process_data():
    """Process some data."""
    return "Processed Data"


# Module-level code that references the package
# This should trigger init function generation
package_ref = mypackage
