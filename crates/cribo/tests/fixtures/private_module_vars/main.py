#!/usr/bin/env python
"""Test case for bundling modules that have private module variables accessed by other modules."""

from pkg.consumer import get_validators

# Test that we can access the private variables through the consumer module
validators = get_validators()
print(f"Got validators: {validators}")

# Verify the structure
assert isinstance(validators, dict)
assert bytes in validators
assert str in validators
assert len(validators[bytes]) == 2
assert len(validators[str]) == 2

print("✓ All tests passed!")
