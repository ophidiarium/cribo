"""Exercise wildcard imports from modules whose __all__ is declared as a tuple."""

from exporter import *


def main():
    print(used())


if __name__ == "__main__":
    main()
