"""Utility functions for the test project."""

import os
import sys
from typing import Union

def greet(name: str) -> str:
    """Greet a person by name."""
    return f"Hello, {name}!"

def calculate(a: Union[int, float], b: Union[int, float]) -> Union[int, float]:
    """Calculate the sum of two numbers."""
    return a + b

def get_system_info() -> dict:
    """Get basic system information."""
    return {
        "platform": sys.platform,
        "python_version": sys.version,
        "cwd": os.getcwd(),
    }
