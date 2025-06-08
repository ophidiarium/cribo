# Test file with unused imports
import os  # Used
import sys  # Unused
import json  # Unused
import math  # Used
from typing import List, Dict, Optional  # List used, Dict and Optional unused
from collections import defaultdict, Counter  # defaultdict used, Counter unused
import re  # Unused


def main():
    # Use os
    print(f"Current directory: {os.getcwd()}")

    # Use math
    result = math.sqrt(16)

    # Use List from typing
    numbers: List[int] = [1, 2, 3, 4]

    # Use defaultdict
    dd = defaultdict(int)
    dd["test"] = 5

    print(f"Square root: {result}")
    print(f"Numbers: {numbers}")
    print(f"Defaultdict: {dict(dd)}")


if __name__ == "__main__":
    main()
