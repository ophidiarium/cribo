"""File utilities module - no side effects, uses aliased stdlib imports."""

import os as py_os
import json as js
from pathlib import Path as PyPath
from datetime import datetime as DT


def get_current_directory() -> str:
    """Get current directory using aliased os module."""
    # Use aliased os module
    cwd = py_os.getcwd()

    # Use aliased Path
    path_obj = PyPath(cwd)
    return str(path_obj.name)


def get_mock_file_info() -> dict:
    """Get mock file information using aliased imports."""
    # Use aliased Path with a fixed path
    path_obj = PyPath("/home/user/test.txt")

    # Use aliased datetime with fixed timestamp for deterministic output
    mod_time = DT.fromtimestamp(1700000000)  # Fixed timestamp: 2023-11-14

    # Create mock data
    info = {
        "size": 1024,
        "name": path_obj.name,
        "parent": str(path_obj.parent),
        "modified": mod_time.isoformat(),
        "exists": True,
        "is_absolute": path_obj.is_absolute(),
    }

    # Use aliased json module for formatting
    return js.loads(js.dumps(info))
