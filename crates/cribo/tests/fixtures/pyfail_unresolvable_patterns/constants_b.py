from constants_a import A_VALUE

# This creates an unresolvable temporal paradox
# B_VALUE depends on A_VALUE being computed first
# But A_VALUE depends on B_VALUE - impossible to resolve
B_VALUE = A_VALUE * 2


def get_b_multiplier():
    return 20
