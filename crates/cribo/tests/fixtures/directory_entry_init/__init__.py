"""Test package with only __init__.py (no __main__.py)."""


def run():
    print("Running from __init__.py as fallback")


# Run when loaded as entry point
run()
