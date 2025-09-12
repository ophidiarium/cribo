"""Child module with initialization code that imports from parent (circular dependency)."""

# This import pattern is common in type checking scenarios
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    # Type checking imports that won't execute at runtime
    from typing import Any

# Import from parent - this creates circular dependency
import main


class ConsoleUser:
    """Class that needs to use parent module's function."""

    def use_console(self):
        """Use the console function from parent module."""
        console = main.get_console()
        return f"Using console: {console}"


# Module-level initialization code
# This kind of init code is common and causes the module to need special handling
_initialized = False


def _init():
    global _initialized
    if not _initialized:
        _initialized = True
        # Initialization that might reference imports
        if TYPE_CHECKING:
            pass


_init()
