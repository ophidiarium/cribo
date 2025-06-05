#!/usr/bin/env python

from services import auth


def main():
    result = auth.authenticate_user("test_user")
    print(f"Relative import cycle result: {result}")


if __name__ == "__main__":
    main()
