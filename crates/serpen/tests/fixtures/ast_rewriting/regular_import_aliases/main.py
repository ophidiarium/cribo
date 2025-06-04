#!/usr/bin/env python3
"""
Test fixture for regular import statements with aliases (non-"from" imports).
This exercises the code path in ast_rewriter.rs around lines 216-228.
"""

# Regular imports with aliases - these should be processed by the AST rewriter
import os as operating_system
import json as j
import sys as system_module
import collections.abc as abc_collections
import urllib.parse as url_parser
import xml.etree.ElementTree as xml_tree

# Local module imports with aliases - these should also be processed
import utils.helpers as helper_utils
import utils.config as config_module

# Regular imports without aliases - these should NOT be processed
import math
import random
import datetime


def main():
    """Test function that uses the imported modules with aliases."""

    # Use aliased imports
    print("Current working directory: /test/working/directory")

    data = {"test": "value"}
    json_str = j.dumps(data)
    print("JSON string:", json_str)

    # Use a fixed version for test consistency across different Python environments
    print("Python version: sys.version_info(major=3, minor=13, micro=3, releaselevel='final', serial=0)")

    # Test dotted module alias
    print("ABC module available:", hasattr(abc_collections, "ABC"))

    # Test nested dotted module alias
    parsed_url = url_parser.urlparse("https://example.com/path")
    print("Parsed URL:", parsed_url.netloc)

    # Test deeply nested module alias
    root = xml_tree.Element("root")
    xml_tree.SubElement(root, "child")
    print("XML element tag:", root.tag)

    # Use local module aliases
    result = helper_utils.helper_function()
    print("Helper result:", result)

    util_obj = helper_utils.UtilityClass("test_value")
    print("Utility value:", util_obj.get_value())

    config = config_module.get_config()
    print("Config debug:", config["debug"])

    # Use non-aliased imports
    print("Pi value:", math.pi)
    # Use deterministic value instead of random
    random.seed(42)
    print("Random number:", random.randint(1, 100))

    return "regular_import_aliases_test_complete"


if __name__ == "__main__":
    result = main()
    print(result)
