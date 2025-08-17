"""Package with circular dependency and __version__ import"""

from .__version__ import __version__, __title__
from .module_a import func_a


# This creates a circular dependency because module_a imports back from package
def package_func():
    return func_a() + " from package"


print(f"Package initialized with version {__version__}")
