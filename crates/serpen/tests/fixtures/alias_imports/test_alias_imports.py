#!/usr/bin/env python3
"""Test file for aliased import first-party detection."""

import sys
import json as js
import test_module as tm  # This should be detected as first-party


def main():
    result = tm.utility_function()
    print(js.dumps({"result": result}))


if __name__ == "__main__":
    main()
