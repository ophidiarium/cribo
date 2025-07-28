"""Consumer module that imports private variables from _internal module."""

# Import private variables directly
from ._internal import (
    _HEADER_VALIDATORS_BYTE,
    _HEADER_VALIDATORS_STR,
    HEADER_VALIDATORS,
    process_headers,
)


def get_validators():
    """Return the validators."""
    # Verify we can access the private variables
    assert _HEADER_VALIDATORS_BYTE is not None
    assert _HEADER_VALIDATORS_STR is not None
    return HEADER_VALIDATORS


def validate_header(header_type, header_data):
    """Use the imported private variables."""
    if header_type == bytes:
        validators = _HEADER_VALIDATORS_BYTE
    else:
        validators = _HEADER_VALIDATORS_STR

    name_re, value_re = validators
    return f"Validated with {name_re.pattern}"
