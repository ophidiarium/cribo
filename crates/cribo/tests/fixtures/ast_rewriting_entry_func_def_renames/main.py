#!/usr/bin/env python3
"""
Test that entry module function definition-time expressions are rewritten
when symbols are renamed due to collisions with submodule symbols.

The entry module defines make_tag, DEFAULT_MODE, and Schema which collide
with helpers.make_tag, helpers.DEFAULT_MODE, and helpers.Schema. The conflict
resolver renames the entry copies. This fixture verifies that:
  1. Decorators referencing renamed symbols are rewritten
  2. Default parameter values referencing renamed symbols are rewritten
  3. Type annotations referencing renamed symbols are rewritten
  4. Function bodies are NOT rewritten (Python resolves at call time)
"""

from helpers import make_tag, DEFAULT_MODE, Schema


# --- Colliding definitions in the entry module ---


def make_tag(func):
    """Entry module's own decorator - collides with helpers.make_tag."""

    def wrapper(*args, **kwargs):
        return f"[entry.tagged] {func(*args, **kwargs)}"

    return wrapper


DEFAULT_MODE = "entry_mode"


class Schema:
    """Entry module's own Schema - collides with helpers.Schema."""

    kind = "entry"

    def __repr__(self):
        return f"Schema(kind={self.kind!r})"


# --- Functions that use the colliding symbols in definition-time expressions ---


@make_tag
def decorated_func(data):
    """Function decorated with the entry module's make_tag."""
    return f"decorated:{data}"


def func_with_default(data, mode=DEFAULT_MODE):
    """Function with default value referencing entry module's DEFAULT_MODE."""
    return f"{mode}:{data}"


def func_with_annotation(data: Schema = None) -> Schema:
    """Function with annotations referencing entry module's Schema."""
    if data is None:
        return Schema()
    return data


# --- Exercise everything ---

print(decorated_func("hello"))
print(func_with_default("world"))
print(func_with_default("world", "custom"))
print(repr(func_with_annotation()))
print(repr(func_with_annotation(Schema())))
