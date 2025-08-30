"""Pretty printing module with circular dependency on console."""

from typing import Any

# Import from abc to use RichRenderable
from .abc import RichRenderable

# Forward reference to console
from .console import Console


class PrettyPrinter(RichRenderable):
    """Pretty printer that implements RichRenderable."""

    def __init__(self):
        self._indent = 2

    def render(self, console: Console) -> str:
        """Render for console output."""
        return f"PrettyPrinter(indent={self._indent})"

    def pretty_print(self, obj: Any) -> None:
        """Pretty print an object."""
        console = Console()
        formatted = format_pretty(obj, console)
        print(formatted)


def format_pretty(obj: Any, console: Console) -> str:
    """Format object for pretty printing."""
    if console.is_renderable(obj):
        return f"<Renderable: {obj}>"
    return repr(obj)
