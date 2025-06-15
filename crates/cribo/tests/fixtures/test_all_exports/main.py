"""
Test fixture for __all__ handling in bundled modules.
This tests that modules with __all__ only expose listed symbols.
"""

from utils import helper_function, UtilityClass


def main():
    """Test that only exported symbols are accessible."""
    # These should work (listed in __all__)
    result = helper_function()
    util = UtilityClass("test")

    print("Helper result:", result)
    print("Utility value:", util.get_value())

    # This should not be accessible (_internal_function not in __all__)
    # Uncomment to test: _internal_function()  # Should raise AttributeError

    return "test_all_exports_complete"


if __name__ == "__main__":
    result = main()
    print(result)
