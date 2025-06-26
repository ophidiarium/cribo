#!/usr/bin/env python3
"""Test global namespace isolation between modules."""

# Main module has its own global variable
foo = "main_foo"
bar = "main_bar"

# Import modules that each have their own globals
import module_global_keyword
import module_globals_dict
import module_mixed_patterns

# Verify main's globals are unchanged
assert foo == "main_foo"
assert bar == "main_bar"

# Call module functions to verify their globals work correctly
print("Module with global keyword:", module_global_keyword.get_foo())
print("Module with globals() dict:", module_globals_dict.get_bar())
print("Module with mixed patterns:", module_mixed_patterns.get_values())

# Verify modules can modify their own globals
module_global_keyword.modify_foo()
module_globals_dict.modify_bar()
module_mixed_patterns.modify_all()

print("\nAfter modifications:")
print("Module with global keyword:", module_global_keyword.get_foo())
print("Module with globals() dict:", module_globals_dict.get_bar())
print("Module with mixed patterns:", module_mixed_patterns.get_values())

# Main's globals should still be unchanged
assert foo == "main_foo"
assert bar == "main_bar"

print("\nMain's globals remain unchanged:")
print(f"foo = {foo}")
print(f"bar = {bar}")
