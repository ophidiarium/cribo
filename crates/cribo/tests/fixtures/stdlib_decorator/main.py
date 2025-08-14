#!/usr/bin/env python3
"""Test contextlib import missing in wrapper module init functions.

This test reproduces the bug found when bundling requests where contextlib
is used as @contextlib.contextmanager decorator but not imported in the
generated init function for wrapper modules with side effects.
"""

import os
import tempfile

# Import from package to trigger processing
from mypackage import utils

# Use the context manager that requires contextlib
# Use platform-independent temporary file path
test_file = os.path.join(tempfile.gettempdir(), "test.txt")
with utils.atomic_open(test_file) as f:
    f.write(b"test")
print("Success")
