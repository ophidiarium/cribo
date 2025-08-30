from . import mod_a

C_CONSTANT = 21


def get_c_value():
    return f"C with A dependency: {mod_a.A_VALUE}"
