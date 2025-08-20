"""Test case for locals/globals shadowing guard.

This test demonstrates the issue where user-defined variables that shadow
the builtin locals or globals functions are incorrectly transformed.
"""


def some_custom_function():
    return {"custom": "result"}


def another_custom_function():
    return {"another": "custom", "result": True}


# Test 1: Use builtin locals() before shadowing (should be transformed)
builtin_locals_result = locals()  # This should be transformed to vars(__cribo_module)

# Test 2: Use builtin globals() before shadowing (should be transformed)
builtin_globals_result = (
    globals()
)  # This should be transformed to __cribo_module.__dict__

# Test 3: Shadow locals with a custom function
locals = some_custom_function
result_locals = locals()  # Should call custom function, not be transformed

# Test 4: Shadow globals with a custom function
globals = another_custom_function
result_globals = globals()  # Should call custom function, not be transformed


def main():
    print("Builtin locals result:", builtin_locals_result)
    print("Builtin globals result:", builtin_globals_result)
    print("Shadowed locals result:", result_locals)
    print("Shadowed globals result:", result_globals)


if __name__ == "__main__":
    main()
