"""Submodule that becomes a wrapper module due to module-level import from parent."""

# Module-level import from parent package
# This causes submodule to become a wrapper module
from . import get_base

# Module-level function call using imported function
# The bundler wraps this in an init function
computed_value = f"computed: {get_base()}"


def process():
    return computed_value
