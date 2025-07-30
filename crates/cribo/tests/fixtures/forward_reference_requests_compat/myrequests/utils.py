"""Utils module that also uses compat."""

from .compat import json, builtin_str


def to_native_string(string):
    """Convert to native string type."""
    if isinstance(string, builtin_str):
        return string
    return str(string)


def decode_json(content):
    """Decode JSON content."""
    try:
        return json.loads(content)
    except (ValueError, TypeError) as e:
        # For the test, just raise our custom JSONDecodeError
        from .exceptions import JSONDecodeError as OurJSONDecodeError

        raise OurJSONDecodeError(str(e))
