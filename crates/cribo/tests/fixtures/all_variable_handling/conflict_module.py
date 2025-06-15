# Module that creates naming conflicts to test __all__ handling
__all__ = ["message", "SHARED_NAME"]

# This creates a conflict with the submodule's message
message = "from conflict_module"

# Variable that might appear in multiple modules
SHARED_NAME = "conflict_module_version"


def internal_func():
    """Internal function not in __all__"""
    return "internal"


# Edge case: what if someone names a variable __all__ (not the special Python __all__)
__all__backup = "this is not the real __all__"
