# Package init that imports submodules
# This creates the namespace structure

from . import compat
from . import exceptions

__all__ = ["compat", "exceptions"]
