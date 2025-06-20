"""Constants module with self-references."""

# Basic constants
MAX_VALUE = 1000
MIN_VALUE = 0
DEFAULT_NAME = "default"

# Self-references of constants (should be removed)
MAX_VALUE = MAX_VALUE  # Should be removed
MIN_VALUE = MIN_VALUE  # Should be removed
DEFAULT_NAME = DEFAULT_NAME  # Should be removed

# Complex constants
CONFIG_DICT = {"max": MAX_VALUE, "min": MIN_VALUE, "name": DEFAULT_NAME}

CONFIG_LIST = [MAX_VALUE, MIN_VALUE, DEFAULT_NAME]

# Self-references of complex constants (should be removed)
CONFIG_DICT = CONFIG_DICT  # Should be removed
CONFIG_LIST = CONFIG_LIST  # Should be removed

# Constants from expressions
COMPUTED_VALUE = MAX_VALUE - MIN_VALUE
COMPUTED_VALUE = COMPUTED_VALUE  # Should be removed

# Tuple constant
LIMITS = (MIN_VALUE, MAX_VALUE)
LIMITS = LIMITS  # Should be removed

# Set constant
VALID_NAMES = {"admin", "user", "guest"}
VALID_NAMES = VALID_NAMES  # Should be removed

# Import and self-reference
from os import path

path = path  # Should be removed

# Conditional constant definition
if MAX_VALUE > 100:
    HIGH_THRESHOLD = True
    HIGH_THRESHOLD = HIGH_THRESHOLD  # Should be removed
else:
    HIGH_THRESHOLD = False
    HIGH_THRESHOLD = HIGH_THRESHOLD  # Should be removed
