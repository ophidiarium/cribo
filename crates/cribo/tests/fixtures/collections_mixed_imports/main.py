"""Test case for mixed collections imports with wrapper modules."""

import module_a
import module_b


def main():
    # Use both modules
    data = module_a.create_ordered_dict()
    result = module_b.check_mapping(data)
    print(f"Result: {result}")


if __name__ == "__main__":
    main()
