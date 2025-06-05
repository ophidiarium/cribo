"""Test case with mixed resolvable and unresolvable circular dependencies."""

from constants_module import BASE_VALUE
from function_module import process_data


def main():
    print(f"Base value: {BASE_VALUE}")
    result = process_data(10)
    print(f"Processed: {result}")


if __name__ == "__main__":
    main()
