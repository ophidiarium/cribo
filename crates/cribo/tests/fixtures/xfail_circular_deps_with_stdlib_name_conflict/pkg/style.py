"""Style module that participates in circular dependencies."""

from typing import Optional

# Create circular dependency with color
from .color import Color


class Style:
    """Style configuration for console output."""

    def __init__(self, color: Optional[str] = None):
        self.color = color or "default"
        self._color_obj = Color()

    def apply(self, text: str) -> str:
        """Apply style to text."""
        return f"[{self.color}]{text}[/{self.color}]"

    def get_color(self) -> Color:
        """Get associated color object."""
        return self._color_obj


# Import console to create more complex circular pattern
def apply_style_to_console(console, style: Style) -> None:
    """Apply a style to console output."""
    from .console import ConsoleBase

    # This creates additional circular reference
    pass
