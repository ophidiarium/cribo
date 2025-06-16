"""Config module that creates unresolvable cycle with constants_module."""

from constants_module import BASE_VALUE

# This creates the unresolvable cycle
CONFIG_MULTIPLIER = 2
DERIVED_VALUE = BASE_VALUE // 2
