"""Module with side effects that assigns builtin types to variables.

This minimal test case reproduces the bug where builtin types are incorrectly
transformed to module.str, module.int, etc. when the bundler wraps modules
with side effects in init functions.
"""

# This print statement is a side effect that forces the bundler to wrap this module
print("Loading compat module...")

# First assign builtins to themselves (like requests.compat does)
# This creates module-level variables named 'str', 'int', etc.
str = str
int = int
bytes = bytes

# Now these assignments will be incorrectly transformed to:
#   builtin_str = module.str  # Error: 'types.SimpleNamespace' object has no attribute 'str'
# because the bundler sees 'str' as a module-level variable from the line above
builtin_str = str
builtin_int = int
builtin_bytes = bytes

# Composite types that reference the module-level variables
basestring = (str, bytes)
numeric_types = (int, float)
