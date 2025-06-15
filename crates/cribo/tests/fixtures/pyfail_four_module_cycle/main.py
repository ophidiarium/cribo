#!/usr/bin/env python

import module_a


def main():
    result = module_a.start_process()
    print(f"Four module cycle result: {result}")


if __name__ == "__main__":
    main()
