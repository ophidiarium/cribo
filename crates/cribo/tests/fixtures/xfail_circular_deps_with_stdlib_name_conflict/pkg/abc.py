"""Module with stdlib-conflicting name that imports from stdlib abc."""

# Import from stdlib abc (this creates the problematic pattern)
from abc import ABC, abstractmethod


class RichRenderable(ABC):
    """Abstract base class for renderables."""

    @abstractmethod
    def render(self, console):
        """Render this object to the console."""
        pass

    @classmethod
    def __subclasshook__(cls, other):
        """Check if a class implements the render protocol."""
        return hasattr(other, "render")


# Import from console to create circular dependency
from .console import ConsoleBase


class RenderableWithConsole(RichRenderable):
    """A renderable that needs a console reference."""

    def __init__(self, console: ConsoleBase):
        self.console = console

    def render(self, console):
        return f"Rendering with {console}"
