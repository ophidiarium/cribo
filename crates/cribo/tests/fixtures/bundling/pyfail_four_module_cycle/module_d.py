from module_a import final_step


def process_in_d():
    """Process in D, depends back on A - completes the 4-module cycle"""
    return f"D({final_step()})"


def step_d():
    return "D_step"
