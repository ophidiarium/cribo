# Module named 'abc' that conflicts with stdlib
from abc import ABC, abstractmethod


class Base(ABC):
    """Base class like rich.abc.RichRenderable"""

    @abstractmethod
    def render(self):
        pass

    @classmethod
    def __subclasshook__(cls, other):
        return hasattr(other, "render")
