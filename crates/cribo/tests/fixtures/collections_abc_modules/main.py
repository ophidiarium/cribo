"""Test case for collections.abc imports hoisting with modules."""

from collections import OrderedDict
from collections.abc import MutableMapping
import helper


def main():
    od = OrderedDict()
    od["a"] = 1
    od["b"] = 2

    result = helper.process_mapping(od)
    print(f"Result: {result}")
    print(f"OrderedDict is MutableMapping: {isinstance(od, MutableMapping)}")


if __name__ == "__main__":
    main()
