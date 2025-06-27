from __future__ import annotations

from mypackage.core import process_data
from mypackage.submodule.utils import validate_input


def main() -> None:
    """Main function with type annotations that require future import."""
    data = {"key": "value", "numbers": [1, 2, 3]}

    if validate_input(data):
        result = process_data(data)
        print(f"Processing result: {result}")
    else:
        print("Invalid input data")


if __name__ == "__main__":
    main()
