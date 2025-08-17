"""Module A that creates circular dependency"""


def func_a():
    # Import package to create circular dependency
    from package import package_func

    return "func_a"


def another_func():
    return "another"
