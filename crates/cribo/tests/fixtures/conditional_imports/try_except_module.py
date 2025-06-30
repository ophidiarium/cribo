"""Module with imports inside try/except blocks."""

# Test case: Import inside try/except
etree = None
ETREE_VERSION = None

try:
    from lxml import etree

    ETREE_VERSION = "lxml"
except ImportError:
    try:
        import xml.etree.ElementTree as etree

        ETREE_VERSION = "stdlib"
    except ImportError:
        # No XML parsing available
        etree = None
        ETREE_VERSION = None

# Another test case: Multiple imports in try block
parser_backend = None
try:
    import cElementTree as ET

    parser_backend = "cElementTree"
except ImportError:
    try:
        import xml.etree.ElementTree as ET

        parser_backend = "ElementTree"
    except ImportError:
        ET = None
        parser_backend = None


# Function using the conditionally imported modules
def parse_xml(xml_string):
    """Parse XML using available parser."""
    if etree is not None:
        return etree.fromstring(xml_string)
    elif ET is not None:
        return ET.fromstring(xml_string)
    else:
        raise ImportError("No XML parser available")


# Module exports
__all__ = ["etree", "ETREE_VERSION", "parse_xml", "ET", "parser_backend"]
