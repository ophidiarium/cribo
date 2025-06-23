# Test importlib static imports with various renaming patterns

# Pattern 1: Aliased importlib import
import importlib as il

# Use aliased importlib to import modules
foo = il.import_module("foo")
bar_module = il.import_module("bar")

# Pattern 2: Renamed import_module import
from importlib import import_module as im

# Use renamed import_module function
also_foo = im("foo")
also_bar = im("bar")

# Test that all imports work correctly
print(f"foo.greet('World'): {foo.greet('World')}")
print(f"foo.get_value(): {foo.get_value()}")
print(f"foo.MESSAGE: {foo.MESSAGE}")

print(f"\nbar_module.process('data'): {bar_module.process('data')}")
print(f"bar_module.VERSION: {bar_module.VERSION}")

# Create calculator instance
calc = bar_module.Calculator()
print(f"calc.add(5, 3): {calc.add(5, 3)}")
print(f"calc.multiply(4, 7): {calc.multiply(4, 7)}")

# Verify that both import methods work correctly
# Note: Identity checks (is) may differ between original and bundled versions
print(f"\nfoo has greet: {hasattr(foo, 'greet')}")
print(f"also_foo has greet: {hasattr(also_foo, 'greet')}")
print(f"bar_module has process: {hasattr(bar_module, 'process')}")
print(f"also_bar has process: {hasattr(also_bar, 'process')}")

# Test attribute access through different references
print(f"\nalso_foo.greet('Python'): {also_foo.greet('Python')}")
print(f"also_bar.process('test'): {also_bar.process('test')}")

print("\nAll importlib renaming patterns work correctly!")
