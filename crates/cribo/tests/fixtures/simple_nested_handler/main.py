import foo
from boo import handler as boo_handler
import pprint


def handler():
    print(foo.handler())
    print(boo_handler())


if __name__ == "__main__":
    handler()
