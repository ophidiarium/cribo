"""Module with hyphen in name - can't be imported with regular import statement."""


class DataProcessor:
    def __init__(self):
        self.name = "Data Processor with Hyphen"

    def process(self, data):
        return f"Processing {data} with hyphenated module"


def get_processor_info():
    return "This is a data processor from a module with a hyphen"


PROCESSOR_VERSION = "1.0-hyphenated"
