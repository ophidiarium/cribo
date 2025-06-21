"""First-party pandera package that shadows the third-party pandera package."""

print("Loading first-party pandera package")

# This is our local pandera package
__version__ = "0.0.1-local-package"


def local_function():
    return "This is from the local pandera package"
