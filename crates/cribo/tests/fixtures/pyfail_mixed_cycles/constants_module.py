"""Module with constants that has unresolvable circular dependency."""

from config_constants import CONFIG_MULTIPLIER

# This creates an unresolvable cycle because it's a module-level constant
BASE_VALUE = 42 * CONFIG_MULTIPLIER


def get_base_value():
    """Function-level access is fine."""
    return BASE_VALUE
