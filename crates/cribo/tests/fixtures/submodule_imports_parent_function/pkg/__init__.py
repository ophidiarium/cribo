"""Package __init__ that gets inlined (not wrapped)."""

# Simple variable initialization (not a side effect)
base_value = "base"


# Function that submodule will import
def get_base():
    return base_value


# Function with lazy import of submodule
# This import happens AFTER pkg.submodule should be initialized
def get_result():
    from .submodule import process

    return process()
