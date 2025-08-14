#!/usr/bin/env python3
"""Test that 'last import wins' semantics work with wildcard imports."""

# Import from two modules with overlapping symbols
from first import *
from second import *


def main():
    # shared_value should be from 'second' module (last import wins)
    assert shared_value() == "from_second", (
        f"Expected 'from_second', got {shared_value()}"
    )
    # unique_first should still be available
    assert unique_first() == "unique_to_first"
    # unique_second should be available
    assert unique_second() == "unique_to_second"

    print("All assertions passed!")
    print(f"shared_value: {shared_value()}")
    print(f"unique_first: {unique_first()}")
    print(f"unique_second: {unique_second()}")


if __name__ == "__main__":
    main()
