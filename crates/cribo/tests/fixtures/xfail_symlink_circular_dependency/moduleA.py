# Module A - the real file
# This file is also accessed as moduleC via symlink


def funcA():
    # Lazy import to avoid circular dependency at module level
    from moduleB import funcB

    return f"A calls {funcB()}"


def get_chain():
    return "A -> B -> C(symlink to A)"


# This will be available when imported as moduleC too
def funcC():
    return "Actually funcA pretending to be funcC"
