#!/usr/bin/env python3
"""
Function name collision test.
Both modules define a function with the same name.
"""

from module_a import process_data as process_a
from module_b import process_data as process_b


def main():
    # Call both functions with the same name from different modules
    result_a = process_a("input from A")
    result_b = process_b("input from B")

    print(f"Module A result: {result_a}")
    print(f"Module B result: {result_b}")

    return {"module_a": result_a, "module_b": result_b}


if __name__ == "__main__":
    result = main()
    print("Result:", result)
