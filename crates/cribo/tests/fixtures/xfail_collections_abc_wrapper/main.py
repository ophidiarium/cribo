#!/usr/bin/env python3
"""Test case for collections.abc imports in wrapper modules."""

from structures import CaseInsensitiveDict

# Create an instance
d = CaseInsensitiveDict()
d["Accept"] = "application/json"
print(f"Accept header: {d['accept']}")  # Should work case-insensitively
print("Success!")
