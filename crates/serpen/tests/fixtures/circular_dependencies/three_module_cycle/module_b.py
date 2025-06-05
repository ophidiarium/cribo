from module_c import process_c


def process_b():
    """Process B that depends on C"""
    return process_c() + "->B"


def get_value_b():
    return "value_from_B"
