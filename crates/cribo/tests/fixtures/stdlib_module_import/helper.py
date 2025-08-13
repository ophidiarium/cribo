"""Helper module that imports stdlib modules at module level."""

# These module-level imports should NOT be transformed to "from X import X"
import abc
import typing
import enum
import threading


def process():
    """Use the imported modules."""

    # Use abc
    class MyBase(abc.ABC):
        pass

    # Use typing
    def typed_func(x: typing.Optional[int]) -> typing.List[str]:
        return []

    # Use enum
    class Color(enum.Enum):
        RED = 1
        GREEN = 2

    # Use threading
    lock = threading.Lock()

    print("All modules used successfully")
    return True
