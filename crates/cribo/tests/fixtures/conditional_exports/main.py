"""Test conditional imports and exports without explicit __all__."""

import compat_module
import exceptions_module

# This should work because JSONDecodeError should be exported from compat_module
# even though it's imported inside an if/else block and there's no __all__
print("compat_module has JSONDecodeError:", hasattr(compat_module, "JSONDecodeError"))

# Test that we can access it
if hasattr(compat_module, "JSONDecodeError"):
    print("✓ Can access compat_module.JSONDecodeError")
else:
    print("✗ Cannot access compat_module.JSONDecodeError")

# Now test using it in a class definition (like requests.exceptions does)
try:

    class CustomJSONError(exceptions_module.BaseError, compat_module.JSONDecodeError):
        """Custom JSON error that inherits from both base error and JSONDecodeError."""

        pass

    print("✓ Successfully created class inheriting from compat_module.JSONDecodeError")
except AttributeError as e:
    print(f"✗ Failed to create class: {e}")

# Test other exports that should be available
expected_exports = [
    "json",  # Imported in try/except
    "JSONDecodeError",  # Imported in if/else
    "builtin_str",  # Regular assignment
    "is_py3",  # Regular assignment
    "chardet",  # Result of function call
]

for export in expected_exports:
    if hasattr(compat_module, export):
        print(f"✓ {export} is accessible")
    else:
        print(f"✗ {export} is NOT accessible")
