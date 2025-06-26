# Submodule with its own __all__
__all__ = ["sub_function", "SUB_CONSTANT"]


def sub_function():
    """Function from submodule"""
    return "Hello from submodule"


def _private_sub_func():
    """Private function in submodule"""
    return "private submodule function"


SUB_CONSTANT = "submodule_value"

# Test potential conflict: variable with same name as in parent
message = "from submodule"
