# Test file deduplication with symlinks

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
# That's why this is an xfail test - it works differently after bundling
