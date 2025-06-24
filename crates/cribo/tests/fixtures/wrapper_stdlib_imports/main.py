#!/usr/bin/env python3
"""Test that stdlib imports work correctly inside wrapper functions."""

from wrapper_module import get_logger

# This should work without NameError
logger = get_logger("test")
print(f"Logger name: {logger.name}")
print("Success: stdlib imports work in wrapper functions!")
