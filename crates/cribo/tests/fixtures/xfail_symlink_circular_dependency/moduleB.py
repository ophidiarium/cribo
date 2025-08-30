# Module B - imports moduleC which is actually a symlink to moduleA
from moduleC import funcC


def funcB():
    # This actually calls funcC from moduleA (via the symlink)
    return f"B calls {funcC()}"
