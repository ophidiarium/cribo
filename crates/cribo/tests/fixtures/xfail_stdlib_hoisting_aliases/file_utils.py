"""File utilities module - no side effects, uses aliased stdlib imports."""

import os as py_os
import json as js
from pathlib import Path as PyPath
from datetime import datetime as DT


def get_current_directory() -> str:
    """Get current directory using aliased os module."""
    # Use a fixed path for deterministic output
    fixed_path = "/home/user/project"

    # Use aliased Path
    path_obj = PyPath(fixed_path)
    return str(path_obj.name)


def get_mock_file_info() -> dict:
    """Get mock file information using aliased imports."""
    # Use aliased Path with a fixed path
    path_obj = PyPath("/home/user/test.txt")

    # Use aliased datetime with fixed ISO string for deterministic output
    mod_time = DT.fromisoformat("2023-11-14T23:13:20")

    # Create mock data with fixed values for cross-platform consistency
    info = {
        "size": 1024,
        "name": "test.txt",  # Fixed value instead of path_obj.name
        "parent": "/home/user",  # Fixed value instead of str(path_obj.parent)
        "modified": mod_time.isoformat(),
        "exists": True,
        "is_absolute": True,  # Fixed value instead of path_obj.is_absolute()
    }

    # Use aliased json module for formatting
    return js.loads(js.dumps(info))
