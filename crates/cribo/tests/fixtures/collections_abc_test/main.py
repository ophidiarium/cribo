"""Test case for collections.abc imports being hoisted properly."""

from collections import OrderedDict
from collections.abc import MutableMapping


def test_func():
    od = OrderedDict()
    od["a"] = 1
    od["b"] = 2

    assert isinstance(od, MutableMapping)
    print("OrderedDict is a MutableMapping:", isinstance(od, MutableMapping))
    print("OrderedDict contents:", dict(od))


if __name__ == "__main__":
    test_func()
