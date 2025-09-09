"""Test that side-effect imports are preserved even when symbols are unused."""

# Global list to track registrations
registered_plugins = []


def test_side_effects():
    """Test function that imports modules for side effects only."""
    # Import the global state from a shared module
    from state import get_state

    # This import should be preserved even though 'register_plugin' symbol is never used
    # because it's an external module that could have side effects
    from external_plugin import register_plugin

    # This import from a bundled module where the symbol is unused should be optimized away
    from utils import unused_function

    # Return the global state to verify side effects
    return get_state()


if __name__ == "__main__":
    from state import get_state

    print(f"Initial state: {get_state()}")
    result = test_side_effects()
    print(f"After imports: {result}")
