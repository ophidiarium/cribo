"""Color module that participates in circular dependencies."""

# Import from style to create circular dependency
import typing
from typing import Optional, Tuple


class Color:
    """Color representation for console output."""

    def __init__(self, r: int = 0, g: int = 0, b: int = 0):
        self.r = r
        self.g = g
        self.b = b

    def to_rgb(self) -> Tuple[int, int, int]:
        """Convert to RGB tuple."""
        return (self.r, self.g, self.b)

    def to_hex(self) -> str:
        """Convert to hex string."""
        return f"#{self.r:02x}{self.g:02x}{self.b:02x}"


# Late import to create circular dependency
def color_from_style(style) -> "Color":
    """Create a color from a style."""
    # Late import to avoid circular import error
    from .style import Style

    # This creates a circular reference with style module
    if style.color == "red":
        return Color(255, 0, 0)
    elif style.color == "green":
        return Color(0, 255, 0)
    elif style.color == "blue":
        return Color(0, 0, 255)
    return Color()


# Also create dependency on console for more complexity
def apply_color_to_console(console, color: Color) -> None:
    """Apply color to console."""
    from .console import ConsoleBase

    pass
