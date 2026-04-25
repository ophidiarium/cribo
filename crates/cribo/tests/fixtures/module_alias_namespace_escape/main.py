"""Ensure module-object reads still preserve the imported namespace surface."""

import package.helper as helper_module


def read_message(module):
    """Read a value from a module object passed through another binding."""
    return module.MESSAGE


def main():
    module_ref = helper_module
    print(read_message(module_ref))


if __name__ == "__main__":
    main()
