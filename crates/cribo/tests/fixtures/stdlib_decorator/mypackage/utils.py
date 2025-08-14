"""Utils module with contextlib decorator.

This module must have side effects to become a wrapper module,
and uses contextlib as a decorator which should be preserved in
the init function but currently isn't.
"""

import contextlib
import os
import tempfile

# Force side effects to make this a wrapper module
print("Loading utils module")


@contextlib.contextmanager
def atomic_open(filename):
    """Write a file atomically using contextlib decorator."""
    print(f"Opening {filename}")
    tmp_descriptor, tmp_name = tempfile.mkstemp(dir=os.path.dirname(filename))
    try:
        with os.fdopen(tmp_descriptor, "wb") as tmp_handler:
            yield tmp_handler
        os.replace(tmp_name, filename)
    except BaseException:
        os.remove(tmp_name)
        raise
