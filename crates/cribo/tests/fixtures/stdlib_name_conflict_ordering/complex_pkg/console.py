# Console module similar to rich.console
from typing import Any, Optional, List
from enum import Enum
from .abc import Base


class RenderMode(Enum):
    DEFAULT = "default"
    HTML = "html"


class Console(Base):
    """Console class similar to rich.console.Console"""

    def __init__(self):
        self.buffer: List[str] = []

    def render(self):
        return "\n".join(self.buffer)

    def print(self, *args: Any):
        self.buffer.append(" ".join(str(arg) for arg in args))
        print(*args)
