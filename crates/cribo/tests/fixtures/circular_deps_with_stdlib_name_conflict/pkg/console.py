"""Console module that has circular dependencies with abc and pretty."""

import sys
from typing import Any, Optional


# Early import to establish base class
class ConsoleBase:
    """Base console class for type checking."""

    pass


# Import from abc to create circular dependency
from .abc import RichRenderable

# Import other modules that will also create circular patterns
from .style import Style
from .color import Color


class Console(ConsoleBase):
    """Main console class with complex dependencies."""

    def __init__(self, force_terminal: Optional[bool] = None):
        self.force_terminal = force_terminal
        self._style = Style()
        self._color = Color()

    def print(self, *objects: Any, **kwargs) -> None:
        """Print objects to console."""
        for obj in objects:
            if isinstance(obj, RichRenderable):
                output = obj.render(self)
            else:
                output = str(obj)
            print(output, **kwargs)

    def is_renderable(self, obj: Any) -> bool:
        """Check if object is renderable."""
        return isinstance(obj, RichRenderable)


# More imports that contribute to circular patterns
from .pretty import format_pretty


def format_with_pretty(console: Console, obj: Any) -> str:
    """Format object using pretty printer."""
    return format_pretty(obj, console)
