"""Test package that has both __main__.py and __init__.py."""

# This should not be used as the entry point when __main__.py exists
print("This is __init__.py - should not run when __main__.py exists")
