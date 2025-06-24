# Test file deduplication with multiple import forms
import importlib

# Import the same module in different ways
import app.utils  # Direct module import
from app import utils as app_utils  # From import with alias

mod = importlib.import_module("app.utils")  # Static importlib call

# All three should reference the same module content
print(f"Direct import: {app.utils.get_name()}")
print(f"From import: {app_utils.get_name()}")
print(f"Importlib: {mod.get_name()}")

# Verify they're all the same
assert app.utils.helper() == app_utils.helper() == mod.helper()
print("SUCCESS: All imports reference the same module!")
