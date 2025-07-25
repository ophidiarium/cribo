from . import mod_c
from .mod_l import LEAF_VALUE

# This is the critical line - accessing attribute on imported module
B_DERIVED_VALUE = mod_c.C_CONSTANT * 2


def get_b_value():
    return f"B using '{LEAF_VALUE}' and derived value '{B_DERIVED_VALUE}'"
