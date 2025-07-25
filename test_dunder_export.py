"""Test Python's export behavior for dunder variables"""

import sys

# Create a test module
test_module_code = """
__version__ = "1.0.0"
_private_var = "private"
public_var = "public"
"""

# Create module
import types

test_module = types.ModuleType("test_module")
exec(test_module_code, test_module.__dict__)

# Check what's accessible
print("Module attributes:", [attr for attr in dir(test_module) if not attr.startswith("__builtins")])
print(f"test_module.__version__ = {test_module.__version__}")
print(f"test_module._private_var = {test_module._private_var}")
print(f"test_module.public_var = {test_module.public_var}")

# Test with __all__
test_module2_code = """
__version__ = "1.0.0"
_private_var = "private"
public_var = "public"
__all__ = ['public_var', '__version__']
"""

test_module2 = types.ModuleType("test_module2")
exec(test_module2_code, test_module2.__dict__)
print("\nWith __all__:")
print("__all__ =", test_module2.__all__)
print("Module attributes:", [attr for attr in dir(test_module2) if not attr.startswith("__builtins")])
