#!/usr/bin/env python3
"""Test script to verify import rewriting functionality."""

# Test case 1: Mixed used/unused imports
import os  # This should be kept
import sys  # This should be removed if unused
import json  # This should be removed if unused

# Test case 2: From imports with mixed usage
from pathlib import Path, PurePath  # Path used, PurePath unused
from collections import defaultdict, Counter  # defaultdict used, Counter unused


def main():
    # Use os and Path
    print(f"Current directory: {os.getcwd()}")
    p = Path(".")
    print(f"Path: {p}")

    # Use defaultdict
    dd = defaultdict(int)
    dd["test"] = 1
    print(f"Defaultdict: {dict(dd)}")


if __name__ == "__main__":
    main()
