# Real file A
# Will also be accessed as link_c and link_e


def start_chain():
    # Import inside function to avoid module-level circular import
    from real_b import from_b

    return f"A -> {from_b()}"


def from_a():
    return "A"


def from_c():
    # This function is called when this file is imported as link_c
    return "C(->A)"


def from_e():
    # This function is called when this file is imported as link_e
    return "E(->A)"


# Unused function that should be removed by tree-shaking
def unused_function_a():
    # This import should also be removed
    from real_b import unused_function_b

    return f"Unused A -> {unused_function_b()}"


# Another unused function with different import pattern
def unused_with_stdlib():
    import json
    import os

    return json.dumps({"path": os.getcwd()})
