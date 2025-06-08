# Package __init__.py with its own __all__ and imports from submodules
from .submodule import sub_function
from .utils import helper_func

__all__ = ["exported_from_init", "sub_function"]


def exported_from_init():
    """Function exported from package __init__.py"""
    return f"From init, using helper: {helper_func()}"


def _internal_init_func():
    """Internal function not exported"""
    return "internal"


# This creates a potential conflict scenario
# where both the package and a module might have __all__
PACKAGE_CONSTANT = "from_package"
