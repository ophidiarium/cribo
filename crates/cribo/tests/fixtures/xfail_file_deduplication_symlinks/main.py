# Test file deduplication with symlinks
#
# This is an xfail test because the bundled behavior differs from regular Python.
# In regular Python, symlinked modules are treated as separate modules.
# After bundling with file deduplication, they share the same module object.

from lib import helpers
from shared import common

# Both should point to the same file
print(f"helpers location: {helpers.get_location()}")
print(f"common location: {common.get_location()}")

# In Python, symlinked modules are treated as separate modules
# They don't share state
print(f"helpers counter: {helpers.increment_counter()}")  # Should be 1
print(f"common counter: {common.increment_counter()}")  # Should be 1 (separate module)

# Verify they DON'T share state in regular Python
assert helpers.counter == 1
assert common.counter == 1
print("SUCCESS: Symlinked modules are separate in Python!")

# But after bundling with file deduplication, they SHOULD share state
# The bundled version will fail this assertion because counter will be 2
# That's why this is an xfail test - it works differently after bundling
