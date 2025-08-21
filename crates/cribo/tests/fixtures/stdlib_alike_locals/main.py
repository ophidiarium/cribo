"""Test fixture for local variables that have the same name as stdlib modules."""

# Import some stdlib modules but not all
import json
import collections
from typing import Any


def process_data():
    """Function that uses local variables named like stdlib modules."""

    # Use the imported json module
    data = {"test": "value"}
    json_str = json.dumps(data)
    print(f"JSON string: {json_str}")

    # Local variable named 'code' (stdlib module we didn't import)
    code = 42
    print(f"Local code variable: {code}")

    # Access attribute on local 'code' variable
    # This should NOT be transformed to _cribo.code
    code_str = str(code)
    print(f"Code as string: {code_str}")

    # Loop variable named after a stdlib module we didn't import
    items = ["a", "b", "c"]
    for code in items:
        # This 'code' is a loop variable, not the stdlib module
        print(f"Loop code: {code}")
        # This should work - accessing string methods
        upper = code.upper()
        print(f"  Uppercase: {upper}")

    # Another stdlib-like name we didn't import
    for socket in range(3):
        # 'socket' is a stdlib module but we're using it as a loop variable
        print(f"Socket number: {socket}")

    # Use imported collections
    counter = collections.Counter(items)
    print(f"Counter: {counter}")

    # Local variable with stdlib name we didn't import
    pickle = "I'm not the pickle module!"
    print(f"Local pickle: {pickle}")

    # Class with attribute access that looks like stdlib
    class StatusCode:
        def __init__(self, value: int, name: str):
            self.value = value
            self._name_ = name

    # Create instances that might be confused with stdlib
    codes = [
        StatusCode(200, "OK"),
        StatusCode(404, "NOT_FOUND"),
        StatusCode(500, "SERVER_ERROR"),
    ]

    # This pattern is similar to what httpx does
    for code in codes:
        # 'code' here is a StatusCode instance, not the stdlib module
        # code._name_ should NOT be transformed to _cribo.code._name_
        print(f"Status {code.value}: {code._name_}")
        # Also test lowercase transformation
        lower_name = code._name_.lower()
        print(f"  Lowercase: {lower_name}")

    # Verify imported modules still work
    data_list = [1, 1, 2, 2, 3]
    counter = collections.Counter(data_list)
    print(f"Counter result: {counter}")

    print("All tests completed!")


def test_shadowing():
    """Test what happens when we shadow an imported module."""
    import os

    # First, use the real os module (check it exists, don't print the path)
    cwd = os.getcwd()
    print(f"os.getcwd() works: {bool(cwd)}")

    # Now shadow it with a local variable
    os = "I'm not the os module!"
    print(f"Shadowed os: {os}")

    # This should use the local string, not try to access os.path
    print(f"Os uppercase: {os.upper()}")


if __name__ == "__main__":
    process_data()
    print("\n--- Testing shadowing ---")
    test_shadowing()
