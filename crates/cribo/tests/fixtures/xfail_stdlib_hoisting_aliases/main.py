#!/usr/bin/env python3
"""Test stdlib hoisting with aliases and renamed imports."""

from file_utils import get_current_directory, get_mock_file_info
from logger import log_message, get_logger_info

# Test the file utilities
dir_name = get_current_directory()
print(f"Current directory name: {dir_name}")

info = get_mock_file_info()
print(f"Mock file info: {info}")

# Test the logger utilities
log_message("Starting application")
logger_info = get_logger_info()
print(f"Logger info: {logger_info}")

print("All tests completed successfully!")
