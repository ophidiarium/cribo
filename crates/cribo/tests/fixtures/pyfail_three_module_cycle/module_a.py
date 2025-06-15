from module_b import process_b


def process_a():
    """Process A that depends on B"""
    return process_b() + "->A"


def get_value_a():
    return "value_from_A"
