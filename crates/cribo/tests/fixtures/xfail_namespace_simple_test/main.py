"""Test namespace imports with simple module"""

from mymodule import utils

# Test using the namespace
print(utils.greet("World"))
print(f"Constant: {utils.CONSTANT}")
