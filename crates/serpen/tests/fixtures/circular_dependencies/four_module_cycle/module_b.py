from module_c import process_in_c


def process_in_b():
    """Process in B, depends on C"""
    return f"B({process_in_c()})"


def step_b():
    return "B_step"
