"""Test case demonstrating import shadowing issue.

This shows how the same import 'utils' resolves to the same top-level
file for all modules, even though package_a and package_b each have
their own utils.py that they might expect to use.

In Python, this behavior is "correct" - when running from the fixture
directory, 'import utils' finds the top-level utils.py for everyone.

But for a bundler, this demonstrates the challenge: without proper
context, it can't know whether 'import utils' in package_a/processor.py
was intended to import package_a/utils.py or the top-level utils.py.
"""

# Import processors from different packages
from package_a.processor import process_a
from package_b.processor import process_b

# Both will use the top-level utils.py
result_a = process_a("test")
result_b = process_b("test")

print(f"Package A result: {result_a}")
print(f"Package B result: {result_b}")

# In this case, they're the same because they both found top-level utils
assert result_a == "TOP_LEVEL_UTILS"
assert result_b == "TOP_LEVEL_UTILS"

print("Both packages used the top-level utils module (as Python does).")
