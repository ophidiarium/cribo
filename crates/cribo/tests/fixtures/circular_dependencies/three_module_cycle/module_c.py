from module_a import get_value_a


def process_c():
    """Process C that depends back on A - creates the cycle"""
    # This creates a function-level circular dependency
    # The cycle is: module_a -> module_b -> module_c -> module_a
    value = get_value_a()
    return f"C(using_{value})"


def get_value_c():
    return "value_from_C"
