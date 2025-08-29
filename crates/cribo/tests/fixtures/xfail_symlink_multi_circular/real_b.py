# Real file B
# Will also be accessed as link_d


def from_b():
    # Import inside function to avoid module-level circular import
    from link_c import from_c

    return f"B -> {from_c()} -> {continue_chain()}"


def continue_chain():
    # Import inside function
    from link_d import from_d

    return from_d()


def from_d():
    # This function is called when this file is imported as link_d
    # Import inside function
    from link_e import from_e

    return f"D(->B) -> {from_e()}"


# Unused function that should be removed by tree-shaking
def unused_function_b():
    return "Unused B"


# Another unused function with symlink import
def unused_with_symlink():
    # Import from symlink that points back to real_a
    from link_e import unused_function_a, from_a

    return f"Unused symlink -> {from_a()}"


# Unused class that should be removed
class UnusedClass:
    def __init__(self):
        # Import inside class method
        from link_c import from_c

        self.value = from_c()

    def get_value(self):
        return self.value
