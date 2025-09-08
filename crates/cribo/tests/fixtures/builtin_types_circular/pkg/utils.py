"""Utils module that matches requests.utils import pattern."""

# Import from multiple modules to create complex dependencies
from . import certs
from .__version__ import __version__
from ._internal_utils import (
    HEADER_VALIDATORS,
    to_native_string,
)

# Import compat module to access its attributes
from . import compat

# Import specific items from compat - exactly like requests.utils
from .compat import (
    basestring,
    bytes,
    integer_types,
    str,
)


def process_data(data):
    """Process data using compat.bytes attribute.

    This will fail when bundled because the bundler skips 'bytes = bytes'
    in compat's init function.
    """
    # This exact line causes AttributeError in bundled requests
    if isinstance(data, compat.bytes):
        return f"Processed {len(data)} bytes"
    elif isinstance(data, compat.str):
        return f"Processed string: {data}"
    else:
        return "Unknown type"
