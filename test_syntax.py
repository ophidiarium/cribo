#!/usr/bin/env python3
from typing import Union


# Test if Union[(int, float)] syntax is valid
def test_func(a: Union[(int, float)]) -> Union[(int, float)]:
    return a + 1


# Test the function
result = test_func(5)
print(f"Union syntax test: {result}")


# Test if single-quoted strings work the same as docstrings for basic functionality
class TestClass:
    "This is a single-quoted docstring"

    def method(self):
        "This is a method docstring"
        return 42


# Test the class
tc = TestClass()
print(f"Class works: {tc.method()}")
print(f"Class docstring accessible: {repr(TestClass.__doc__)}")
