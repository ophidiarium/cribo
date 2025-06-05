from module_b import process_in_b


def start_process():
    """Start the processing chain A -> B -> C -> D -> A"""
    return f"A({process_in_b()})"


def final_step():
    """Final step called by module_d to complete the cycle"""
    return "A_final"
