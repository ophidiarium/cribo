#!/usr/bin/env python3
"""Simple test file to verify aliased import detection."""

import test_module as tm


def main():
    result = tm.utility_function()
    print(result)


if __name__ == "__main__":
    main()
