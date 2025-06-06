#!/usr/bin/env python3
"""Test late future imports - these should trigger F404 rule"""

# Regular imports first
import os
import sys
from pathlib import Path

# This future import is late - should trigger F404
from __future__ import annotations


def main():
    """Main function with future annotations"""
    name: str = "test"
    print(f"Hello {name}")
    print(f"from: {Path(__file__).name}")


if __name__ == "__main__":
    main()
