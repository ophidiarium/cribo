from module_d import process_in_d


def process_in_c():
    """Process in C, depends on D"""
    return f"C({process_in_d()})"


def step_c():
    return "C_step"
