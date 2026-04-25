"""Export a single public symbol through tuple-based __all__."""

__all__ = ("used",)


def used():
    """Return the symbol that should survive wildcard import tree-shaking."""
    return "used from tuple __all__"


def unused():
    """This should be dropped from the bundled output."""
    return "unused from tuple __all__"
