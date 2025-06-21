"""First-party module that shadows the third-party pandera package."""

print("Loading first-party pandera module")

# This is our local pandera module
__version__ = "0.0.1-local"


def local_function():
    return "This is from the local pandera module"
