# Simple test with completely used or unused imports
import os  # Used
import sys  # Unused
import json  # Unused
import math  # Used


def main():
    # Use os
    print(f"Current directory: {os.getcwd()}")

    # Use math
    result = math.sqrt(16)
    print(f"Square root: {result}")


if __name__ == "__main__":
    main()
