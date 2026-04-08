"""
Helper module that defines symbols colliding with the entry module.
The conflict resolver will rename one copy; this fixture verifies that
definition-time expressions (decorators, defaults, annotations) in entry
module functions are rewritten to use the renamed identifiers.
"""


def make_tag(func):
    """Decorator that tags a function's return value."""

    def wrapper(*args, **kwargs):
        return f"[helpers.tagged] {func(*args, **kwargs)}"

    return wrapper


DEFAULT_MODE = "helpers_mode"


class Schema:
    """Type used for annotations."""

    kind = "helpers"

    def __repr__(self):
        return f"Schema(kind={self.kind!r})"
