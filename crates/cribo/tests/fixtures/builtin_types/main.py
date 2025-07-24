"""Test that builtin type assignments work in modules with side effects."""

from compat_module import builtin_str, builtin_int, basestring, numeric_types

# These assertions should pass but will fail due to the bundler bug
assert builtin_str is str
assert builtin_int is int

assert builtin_str("test") == "test"
assert builtin_int("42") == 42

assert basestring == (str, bytes)
assert numeric_types == (int, float)

print("All tests passed!")
