"""Test symbol usage optimization with circular dependencies."""

from module_a import FunctionA


def main():
    result = FunctionA().compute()
    print(result)


if __name__ == "__main__":
    main()
