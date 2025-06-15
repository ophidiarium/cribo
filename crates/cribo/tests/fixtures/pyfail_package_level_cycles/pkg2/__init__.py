from pkg1 import utility_function


def helper_function():
    """Helper function that depends on pkg1"""
    util_result = utility_function()
    return f"pkg2.helper(using_{util_result})"


def another_helper():
    """Another function in pkg2"""
    return "pkg2_helper"
