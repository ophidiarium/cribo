"""Test f-string transformation with module globals lifting"""

from worker import Worker


def main():
    w = Worker()

    # This should work correctly with lifted globals
    result = w.process("test")
    print(result)

    # Check the worker's state
    print(w.get_status())

    # Perform work
    w.do_work()
    print(w.get_status())


if __name__ == "__main__":
    main()
