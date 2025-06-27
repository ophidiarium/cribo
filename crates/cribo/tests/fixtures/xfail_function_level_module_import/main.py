#!/usr/bin/env python


def process_data():
    # Import a module (not a value) from within a function
    from utils import calculator

    result = calculator.add(5, 3)
    print(f"Result: {result}")
    print(f"Calculator description: {calculator.description}")


if __name__ == "__main__":
    process_data()
