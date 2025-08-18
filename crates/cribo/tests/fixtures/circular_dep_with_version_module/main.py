#!/usr/bin/env python3
"""Test case for circular dependency with __version__ module import"""

import package

print(f"Package version: {package.__version__}")
print(f"Package title: {package.__title__}")
print("Success!")
