"""Module named 'for' - a Python keyword for loops."""


class ForLoop:
    def __init__(self, iterable):
        self.iterable = iterable
        self.index = 0

    def iterate(self):
        results = []
        for item in self.iterable:
            results.append(f"Processing: {item}")
        return results


def create_loop(items):
    return ForLoop(items)


LOOP_TYPES = ["for", "while", "comprehension"]
MAX_ITERATIONS = 1000
