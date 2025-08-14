"""Test case for wrapper module initialization in function scopes.

This reproduces the issue where wrapper module initialization statements
are inserted into function bodies and cause NameError or variable shadowing.
"""


def test_function():
    # Function-scoped import that triggers wrapper module initialization
    # This should not cause NameError when the wrapper module init
    # statements are inserted into this function body
    from pkg.submodule import some_function

    result = some_function("test")
    return result


# This should work without errors
result = test_function()
print(f"Result: {result}")
print("Success!")
