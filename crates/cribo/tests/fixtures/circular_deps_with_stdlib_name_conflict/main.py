"""Test circular dependencies with stdlib-conflicting module names.

This fixture tests the bundler's ability to handle:
1. A local module named 'abc' that conflicts with stdlib 'abc'
2. Circular dependencies that require wrapper functions
3. Proper stdlib normalization when hard dependencies are involved
4. Ensuring no incorrect imports are generated for stdlib or bundled modules
"""

from pkg.console import Console
from pkg.pretty import PrettyPrinter

# Initialize console and use it
console = Console()
console.print("Hello from main!")

# Use pretty printer that depends on abc
pp = PrettyPrinter()
pp.pretty_print({"test": "data"})

# Verify that abc module is properly initialized
from pkg.abc import RichRenderable

print(f"RichRenderable class: {RichRenderable.__module__}.{RichRenderable.__name__}")
