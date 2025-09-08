"""Test parent-child circular dependencies.

This tests the case where:
1. foo/__init__.py imports from foo.boo
2. foo.boo imports foo.zoo
3. This creates a circular dependency because foo.zoo requires foo to be initialized first
"""

# Side effect to track module initialization order
print("Initializing main module")

# Import from the package - this should trigger the full initialization chain
from foo.boo import process_data

# Use the imported function
result = process_data("test input")
print(f"Result: {result}")
